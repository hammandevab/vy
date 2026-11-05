//! A tiny in-process HTTP/1.1 server for tests: one connection at a time,
//! `Connection: close`, records every request. No network beyond loopback.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// A recorded request.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    /// The body, as far as `Content-Length` says (at most 64 KB here).
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// What to send back.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Self {
        Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn ok(content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Response::new(200)
            .header("Content-Type", content_type)
            .body(body)
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

type Handler = Box<dyn FnMut(&Request) -> Response + Send>;

pub struct TestServer {
    port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    /// Start on an ephemeral loopback port.
    pub fn start(handler: impl FnMut(&Request) -> Response + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut handler: Handler = Box::new(handler);
        let thread = {
            let requests = Arc::clone(&requests);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // The listener is non-blocking so the loop can
                            // see `stop`; on the BSD sockets of macOS an
                            // accepted socket inherits that flag (Linux
                            // does not), and then a read before the request
                            // has arrived, or a write of a body larger than
                            // the socket buffer, fails with WouldBlock: no
                            // response, or a cut one, and the client sees
                            // "Peer disconnected" at random. Blocking is what
                            // the handler below is written for.
                            let _ = stream.set_nonblocking(false);
                            if let Some(req) = read_request(&stream) {
                                let resp = handler(&req);
                                requests.lock().unwrap().push(req);
                                write_response(&stream, &resp);
                            }
                            // Close the sending side first and drain whatever the
                            // client still sends (an unread POST body): closing a
                            // socket with unread data makes the kernel send RST,
                            // and on macOS the client then sees "Peer disconnected"
                            // before it has read our response.
                            let _ = stream.shutdown(Shutdown::Write);
                            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                            let mut sink = [0u8; 4096];
                            while let Ok(n) = (&stream).read(&mut sink) {
                                if n == 0 {
                                    break;
                                }
                            }
                            let _ = stream.shutdown(Shutdown::Both);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        TestServer {
            port,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    pub fn count(&self, path: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.path == path && r.method == "GET")
            .count()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn read_request(mut stream: &TcpStream) -> Option<Request> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let rest;
    loop {
        if let Some(end) = find_header_end(&buf) {
            rest = buf.split_off(end + 4);
            buf.truncate(end);
            break;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 64 * 1024 {
            return None;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split(' ');
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0)
        .min(64 * 1024);
    let mut body = rest;
    while body.len() < length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn write_response(mut stream: &TcpStream, resp: &Response) {
    let reason = match resp.status {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", resp.status);
    for (n, v) in &resp.headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    if resp.status != 304 {
        head.push_str(&format!("Content-Length: {}\r\n", resp.body.len()));
    }
    head.push_str("Connection: close\r\n\r\n");
    let _ = stream.write_all(head.as_bytes());
    if resp.status != 304 {
        let _ = stream.write_all(&resp.body);
    }
    let _ = stream.flush();
}
