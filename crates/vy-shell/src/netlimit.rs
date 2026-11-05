//! A tab's network budget as the main process holds it (SPEC-9.3.1).
//!
//! The tab process's runtime keeps the same limits for the page, but the
//! tab runs page code and is not trusted to (review 0004, finding 2), so
//! the main process, which performs every request, counts again: at most
//! [`MAX_OPEN`] requests in flight per tab and [`BYTES_PER_MINUTE`] of URLs,
//! request bodies and response bodies per minute of the shell's clock. A
//! request over either is answered `E_RATE` and never made; a response
//! that does not fit is delivered as `E_RATE` without its body.

use vy_abi::err;

/// Requests in flight per tab (SPEC-9.3.1, `vy_abi::limits::MAX_OPEN_FETCHES`).
pub const MAX_OPEN: u32 = vy_abi::limits::MAX_OPEN_FETCHES;
/// Network bytes per minute per tab (SPEC-9.3.1).
pub const BYTES_PER_MINUTE: u64 = vy_runtime::NET_BYTES_PER_MINUTE;

/// One tab's budget. The byte window lives as long as the tab, across
/// navigations, so a page cannot reset it by reloading itself; the count
/// of open requests is per page ([`NetBudget::new_page`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetBudget {
    window_start_ms: i64,
    bytes: u64,
    open: u32,
}

impl NetBudget {
    fn roll(&mut self, now_ms: i64) {
        if now_ms.saturating_sub(self.window_start_ms) >= 60_000 {
            self.window_start_ms = now_ms;
            self.bytes = 0;
        }
    }

    fn admit(&mut self, now_ms: i64, bytes: u64) -> bool {
        self.roll(now_ms);
        match self.bytes.checked_add(bytes) {
            Some(total) if total <= BYTES_PER_MINUTE => {
                self.bytes = total;
                true
            }
            _ => false,
        }
    }

    /// A request of `bytes` (URL and body) at `now_ms`: counted and open,
    /// or `Err(E_RATE)` and nothing counted.
    pub fn begin(&mut self, now_ms: i64, bytes: u64) -> Result<(), i32> {
        if self.open >= MAX_OPEN || !self.admit(now_ms, bytes) {
            return Err(err::E_RATE);
        }
        self.open += 1;
        Ok(())
    }

    /// The response to an open request, of `bytes`: true when it fits the
    /// minute and is delivered, false when it is to be answered `E_RATE`
    /// without its body. Either way the request is no longer open.
    pub fn complete(&mut self, now_ms: i64, bytes: u64) -> bool {
        self.open = self.open.saturating_sub(1);
        self.admit(now_ms, bytes)
    }

    /// A new page in the tab: its predecessor's requests are dropped
    /// unanswered, so none of them is open any more.
    pub fn new_page(&mut self) {
        self.open = 0;
    }

    /// Requests in flight.
    pub fn open(&self) -> u32 {
        self.open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-9.3.1: in the main process, the 65th request in flight is
    /// `E_RATE` until one is answered; a minute carries 256 MB in all,
    /// requests that do not fit are refused uncounted, responses that do
    /// not fit are not delivered; the next minute starts afresh; a new
    /// page in the tab closes the old page's requests but not the minute.
    #[test]
    fn spec_9_3_1_main_process_net_limits() {
        let mut b = NetBudget::default();
        for _ in 0..MAX_OPEN {
            assert_eq!(b.begin(0, 100), Ok(()));
        }
        assert_eq!(b.begin(0, 100), Err(err::E_RATE), "the 65th in flight");
        assert!(b.complete(10, 1000));
        assert_eq!(b.begin(10, 100), Ok(()), "room again");
        assert_eq!(b.open(), MAX_OPEN);
        b.new_page();
        assert_eq!(b.open(), 0);

        let used = 65 * 100 + 1000;
        let rest = BYTES_PER_MINUTE - used;
        assert!(!b.complete(20, rest + 1), "a response over the minute");
        assert_eq!(b.begin(20, rest), Ok(()), "exactly the rest fits");
        assert_eq!(b.begin(30, 1), Err(err::E_RATE), "the minute is full");
        assert_eq!(b.open(), 1, "a refused request is not open");
        assert!(!b.complete(59_999, 1));
        assert_eq!(b.begin(60_000, BYTES_PER_MINUTE), Ok(()), "a new minute");
        assert_eq!(b.begin(60_001, u64::MAX), Err(err::E_RATE));
    }
}
