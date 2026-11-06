//! `start.json`, the restart ledger: a writer that crashes at start cannot be
//! restarted every 5 s by every mod. Each spawn is counted before it happens,
//! and only a proven start removes the count.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::files::Paths;

const NAME: &str = "start.json";
/// The failure in a row that disables the writer.
pub const MAX_FAILURES: u32 = 8;
const FIRST: Duration = Duration::from_secs(5);
const LONGEST: Duration = Duration::from_secs(160);

/// The ledger.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    v: u8,
    /// Failures in a row, this attempt included.
    pub failures: u32,
    /// No spawn before this, Unix milliseconds.
    pub retry_at_ms: u64,
}

/// How long failure `n` defers the next spawn: min(5 s × 2^(n−1), 160 s).
pub fn backoff(n: u32) -> Duration {
    FIRST
        .saturating_mul(1 << n.saturating_sub(1).min(6))
        .min(LONGEST)
}

/// Now, Unix milliseconds.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Ledger {
    /// The ledger of `paths`; absent is no failures. One that cannot be read
    /// stops starts: dropping it would drop the bound.
    pub fn read(paths: &Paths) -> Result<Ledger, String> {
        match std::fs::read(paths.file(NAME)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .ok()
                .filter(|l: &Ledger| l.v == 1)
                .ok_or_else(|| "start.json cannot be read; reinstall to clear it".to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
            Err(e) => Err(format!("start.json: {e}")),
        }
    }

    /// How long a spawn is still deferred. A date past the longest backoff is
    /// a clock that went back, and counts as passed.
    pub fn deferred(&self, now_ms: u64) -> Option<Duration> {
        let left = Duration::from_millis(self.retry_at_ms.checked_sub(now_ms)?);
        (!left.is_zero() && left <= LONGEST).then_some(left)
    }

    /// Counts one more spawn, before it happens.
    pub fn count(&self, paths: &Paths, now_ms: u64) -> Result<Ledger, String> {
        let failures = self.failures + 1;
        let wait = u64::try_from(backoff(failures).as_millis()).unwrap_or(u64::MAX);
        let ledger = Ledger {
            v: 1,
            failures,
            retry_at_ms: now_ms.saturating_add(wait),
        };
        let bytes = serde_json::to_vec(&ledger).map_err(|e| e.to_string())?;
        paths
            .replace(NAME, ".herdr-inbox-start.tmp", &bytes)
            .map_err(|e| format!("could not count the start: {e}"))?;
        Ok(ledger)
    }
}

/// A proven start: the run of failures is over.
pub fn clear(paths: &Paths) {
    let _ = paths.remove(NAME);
}

#[cfg(test)]
mod tests;
