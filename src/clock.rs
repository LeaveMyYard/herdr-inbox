//! UTC timestamps in the format the inbox stores: Python's
//! `datetime.now(timezone.utc).isoformat(timespec="seconds")`, for example
//! `2026-10-18T09:05:03+00:00` (`store.now`). The stored values are compared
//! as strings, so the format must match to the character.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `t` in the stored format, its fraction of a second dropped (as
/// `timespec="seconds"` truncates).
pub fn iso_seconds(t: SystemTime) -> String {
    let secs = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => {
            let d = e.duration();
            -(d.as_secs() as i64) - i64::from(d.subsec_nanos() > 0)
        }
    };
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Now, in the stored format (Python `store.now()`).
pub fn now() -> String {
    iso_seconds(SystemTime::now())
}

/// `ago` before now, in the stored format (Python `store.now(-ago)`).
pub fn now_minus(ago: Duration) -> String {
    iso_seconds(SystemTime::now() - ago)
}

/// The proleptic Gregorian (year, month, day) of a day count from 1970-01-01
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests;
