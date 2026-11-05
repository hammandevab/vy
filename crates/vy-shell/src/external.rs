//! Opening `https://` links outside the client (SPEC-2.1.7): the client
//! never renders them, the system browser does. Nothing else leaves the
//! client this way: only an `https://` URL made of printable ASCII is
//! handed over, as one argument, to the platform's opener, and never
//! through a shell that could read anything in it.

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

/// The platform's opener: `ShellExecuteW` on Windows, `open` on macOS,
/// `xdg-open` elsewhere. On every platform the URL is one argument of
/// one program; no command interpreter is in between. (`cmd /C start`
/// was: `cmd.exe` reads `&` as a command separator and `%NAME%` as an
/// environment variable in an unquoted argument, and `std::process`
/// quotes an argument only when it has a space, which an `https://` URL
/// never has – so a page could have run a command with `navigate`.)
#[derive(Debug, Default)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&mut self, url: &str) -> Result<(), String> {
        if !is_external(url) {
            return Err(format!("{url:?} is not an https:// address"));
        }
        platform::open(url)
    }
}

#[cfg(windows)]
mod platform {
    /// `ShellExecuteW(NULL, "open", url, NULL, NULL, SW_SHOWNORMAL)`: the
    /// URL goes to the handler registered for `https` as one string.
    pub fn open(url: &str) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &str| -> Vec<u16> {
            std::ffi::OsStr::new(s)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        };
        let verb = wide("open");
        let file = wide(url);
        // SAFETY: both pointers are to NUL-terminated UTF-16 buffers that
        // outlive the call; null for the window, the parameters and the
        // directory is what the API documents for "none".
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // Documented contract: a value above 32 means success.
        if result as usize > 32 {
            Ok(())
        } else {
            Err(format!(
                "ShellExecuteW failed with code {}",
                result as usize
            ))
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::process::{Command, Stdio};

    /// `open` on macOS, `xdg-open` elsewhere, with the URL as its one
    /// argument (an `argv` element, never a shell line).
    pub fn open(url: &str) -> Result<(), String> {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        Command::new(program)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
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
