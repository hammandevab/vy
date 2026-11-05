//! Opening `https://` links outside the client (SPEC-2.1.7): the client
//! never renders them, the system browser does. Nothing else leaves the
//! client this way: only an `https://` URL made of printable ASCII is
//! handed over, as one argument, to the platform's opener.

use std::process::{Command, Stdio};

/// Something that opens an `https://` URL elsewhere.
pub trait Opener {
    /// Open `url`; the error is for the log, the user sees the address.
    fn open(&mut self, url: &str) -> Result<(), String>;
}

/// True for the only kind of URL the opener accepts: `https://` and
/// printable ASCII without the characters a URL never carries unencoded
/// and a command line might read (`"`, `<`, `>`, `\`, `^`, `|`, `` ` ``,
/// `{`, `}`).
pub fn is_external(url: &str) -> bool {
    url.starts_with("https://")
        && url.len() > "https://".len()
        && url
            .bytes()
            .all(|b| (0x21..0x7f).contains(&b) && !b"\"<>\\^|`{}".contains(&b))
}

/// The platform's opener: `cmd /C start` on Windows, `open` on macOS,
/// `xdg-open` elsewhere.
#[derive(Debug, Default)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&mut self, url: &str) -> Result<(), String> {
        if !is_external(url) {
            return Err(format!("{url:?} is not an https:// address"));
        }
        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            // `start` takes the first quoted argument as a window title;
            // the URL has no quotes (not printable-ASCII safe) and is
            // quoted so `&` in a query is not a command separator.
            c.args(["/C", "start", "", url]);
            c
        } else if cfg!(target_os = "macos") {
            let mut c = Command::new("open");
            c.arg(url);
            c
        } else {
            let mut c = Command::new("xdg-open");
            c.arg(url);
            c
        };
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

/// An opener that only remembers what it was asked to open, for tests.
#[derive(Debug, Default)]
pub struct RecordingOpener {
    /// Every URL opened, in order.
    pub opened: Vec<String>,
}

impl Opener for RecordingOpener {
    fn open(&mut self, url: &str) -> Result<(), String> {
        if !is_external(url) {
            return Err(format!("{url:?} is not an https:// address"));
        }
        self.opened.push(url.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-2.1.7: only printable `https://` addresses go to the system
    /// browser; `vy://`, `http://`, `file:` and anything with spaces,
    /// quotes or control characters never leave the client.
    #[test]
    fn spec_2_1_7_only_https_leaves_the_client() {
        assert!(is_external("https://example.test/a?b=c&d=e"));
        for bad in [
            "vy://example.test/",
            "http://example.test/",
            "https://",
            "file:///etc/passwd",
            "https://example.test/a b",
            "https://example.test/\"",
            "https://example.test/\n",
            "javascript:alert(1)",
            "https://exämple.test/",
        ] {
            assert!(!is_external(bad), "{bad}");
            assert!(RecordingOpener::default().open(bad).is_err(), "{bad}");
        }
        let mut rec = RecordingOpener::default();
        rec.open("https://example.test/x").unwrap();
        assert_eq!(rec.opened, vec!["https://example.test/x"]);
    }
}
