//! ISO 8601 timestamps as the inbox stores them (Python
//! `datetime.isoformat(timespec="seconds")`: `2026-10-17T12:00:00+00:00`),
//! read back to Unix seconds for the popup's ages.

/// `YYYY-MM-DD[T ]HH:MM:SS[.fraction][Z|±HH:MM]` as Unix seconds (no
/// offset: UTC); `None` if it does not parse.
pub(crate) fn unix_seconds(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = (s.get(..10)?, s.get(10..)?);
    let mut d = date.split('-');
    let (y, m, day) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    let rest = rest.strip_prefix(['T', ' ']).unwrap_or(rest);
    let time = rest.get(..8).unwrap_or("00:00:00");
    let mut t = time.split(':');
    let (hh, mm, ss) = (num(t.next()?)?, num(t.next()?)?, num(t.next()?)?);
    let mut zone = rest.get(8..).unwrap_or("");
    if let Some(frac) = zone.strip_prefix('.') {
        zone = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match zone {
        "" | "Z" => 0,
        z => {
            let sign = match z.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let mut o = z[1..].split(':');
            sign * (num(o.next()?)? * 3600 + num(o.next().unwrap_or("0"))? * 60)
        }
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(y, m, day) * 86400 + hh * 3600 + mm * 60 + ss - offset)
}

fn num(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
