//! Claim tokens (Python `uuid.uuid4().hex[:8]`): 8 random hex digits.

use std::fs::File;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

/// 8 lowercase hex digits from the system's random source. Without one
/// (unreadable `/dev/urandom`), from the clock and the process id: a claim
/// token only has to differ from a concurrent claimer's.
pub fn hex8() -> String {
    let mut bytes = [0u8; 4];
    let random = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    let value = match random {
        Ok(()) => u32::from_le_bytes(bytes),
        Err(_) => {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos());
            nanos ^ std::process::id().rotate_left(16)
        }
    };
    format!("{value:08x}")
}

#[cfg(test)]
mod tests;
