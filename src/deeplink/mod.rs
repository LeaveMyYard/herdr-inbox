//! Opening the popup on one question or group (Python `deeplink.py`), the
//! integration API for other plugins.
//!
//! `herdr plugin pane open` cannot pass arguments to the pane command, so
//! `inbox open --question Qn | --group G | --pane P | --session S` writes a
//! one-shot focus request next to the database and opens the popup, which
//! takes it on start:
//!
//! - [`request`]: the request `{kind, value, created_at, nonce}`, written
//!   atomically (the newest wins) and taken once, by the popup holding its
//!   nonce, within [`request::REQUEST_TTL`];
//! - [`resolve`]: the open group (and question) a request names, or none;
//! - [`lock`]: the running popup's lock, so `inbox open` can say the popup
//!   is already open instead of leaving a request nobody takes;
//! - [`qref`]: question references (`Q12`, `qnap:Q12`).
//!
//! The popup does it in Python's order (`tui.on_mount`): take the lock
//! ([`lock::hold_popup_lock`]) first, then the request
//! ([`request::consume_request`]), then [`resolve::resolve`] it. Taking the
//! lock first narrows the window in which another `inbox open` sees no popup
//! and writes a request this one would take and drop.

pub mod lock;
pub mod qref;
pub mod request;
pub mod resolve;

use herdr_agent_kit::agent_session::SessionId;
use unicode_general_category::{GeneralCategory, get_general_category};

use crate::text::py_isspace;
use qref::question_ref;

/// What a request can name, in `inbox open`'s order (Python `deeplink.KINDS`).
pub const KINDS: [&str; 4] = ["question", "group", "pane", "session"];
/// `inbox open` hands its request's nonce to the popup it opens in this
/// variable (Python `deeplink.NONCE_ENV`).
pub const NONCE_ENV: &str = "INBOX_OPEN_NONCE";

/// What a request asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// One of [`KINDS`].
    pub kind: String,
    /// What it names (`12`, `qnap:12`, `w1:p2`, a session id).
    pub value: String,
    /// Hold every answering key until a deliberate Tab/→ (`--no-hold`
    /// lifts it: an operator's own key or click opened the inbox).
    pub hold: bool,
}

/// `value` is a well-formed `kind` (Python `deeplink.valid`): a question
/// `[machine:][Qq]n`, a group `n` (Unicode digits, as Python's `\d`), a
/// pane any word without whitespace, a session a canonical session id.
pub fn valid(kind: &str, value: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.chars().all(is_nd);
    match kind {
        "question" => {
            let (machine, rest) = match value.find(':') {
                Some(i) => (Some(&value[..i]), &value[i + 1..]),
                None => (None, value),
            };
            let machine_ok = machine.is_none_or(|m| {
                (1..=64).contains(&m.len())
                    && m.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            });
            let number = rest.strip_prefix(['Q', 'q']).unwrap_or(rest);
            machine_ok && digits(number)
        }
        "group" => digits(value),
        "pane" => !value.is_empty() && !value.chars().any(py_isspace),
        "session" => SessionId::parse(value).is_some(),
        _ => false,
    }
}

/// How a notice names a request's target (Python `deeplink.describe`):
/// `Q12`, `qnap:Q12`, `group 3`, `pane w1:p2`.
pub fn describe(kind: &str, value: &str) -> String {
    if kind == "question" {
        let r = question_ref(value);
        let (machine, number) = match r.rfind(':') {
            Some(i) => (&r[..=i], &r[i + 1..]),
            None => ("", r.as_str()),
        };
        return format!("{machine}Q{number}");
    }
    format!("{kind} {value}")
}

fn is_nd(c: char) -> bool {
    get_general_category(c) == GeneralCategory::DecimalNumber
}

#[cfg(test)]
mod tests;
