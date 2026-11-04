//! pydantic's lax-mode readings of a JSON argument (the Python server validates
//! every call against the signature): a string stays a string, an integer may
//! come as a whole float or a numeric string, a boolean as 0/1 or a word. What
//! it refuses is a [`Refusal`]: the field and why.

use serde_json::Value;

/// Why an argument was refused: the field's name and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The argument (a path such as `questions.0.question` inside a list).
    pub field: String,
    /// What was wrong.
    pub why: &'static str,
}

impl Refusal {
    /// `field` refused for `why`.
    pub fn new(field: impl Into<String>, why: &'static str) -> Refusal {
        Refusal {
            field: field.into(),
            why,
        }
    }
}

/// Every refusal of one call, in pydantic's order.
#[derive(Debug, Default)]
pub struct Refusals(Vec<Refusal>);

impl Refusals {
    /// Adds one.
    pub fn push(&mut self, r: Refusal) {
        self.0.push(r);
    }

    /// Adds all of `other`'s.
    pub fn extend(&mut self, other: Refusals) {
        self.0.extend(other.0);
    }

    /// How many so far.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// None so far.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The value, or `None` with its refusal kept.
    pub fn take<T>(&mut self, r: Result<T, Refusal>) -> Option<T> {
        r.map_err(|e| self.0.push(e)).ok()
    }

    /// `value` if nothing was refused, else every refusal.
    pub fn done<T>(self, value: T) -> Result<T, Vec<Refusal>> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(self.0)
        }
    }
}

/// A required field that is absent.
pub const MISSING: &str = "Field required";

/// `str`: only a JSON string.
pub fn string(v: &Value, field: &str) -> Result<String, Refusal> {
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| Refusal::new(field, "Input should be a valid string"))
}

/// `str | None`: absent and `null` are `None`.
pub fn opt_string(v: Option<&Value>, field: &str) -> Result<Option<String>, Refusal> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => string(v, field).map(Some),
    }
}

/// `int`: an integer, `true`/`false`, a float with no fraction, or a string of
/// one (pydantic strips the string's whitespace, allows `_` between digits as
/// `int()` does, and a fraction of zeros). Beyond `i64` is refused: no id is.
pub fn int(v: &Value, field: &str) -> Result<i64, Refusal> {
    let bad = |why| Refusal::new(field, why);
    match v {
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Ok(i),
            (None, Some(f)) if f.fract() != 0.0 => Err(bad(
                "Input should be a valid integer, got a number with a fractional part",
            )),
            (None, Some(f)) if f.abs() < 9.2e18 => Ok(f as i64),
            _ => Err(bad(
                "Unable to parse input string as an integer, exceeded maximum size",
            )),
        },
        Value::String(s) => int_text(s.trim()).ok_or_else(|| {
            bad("Input should be a valid integer, unable to parse string as an integer")
        }),
        _ => Err(bad("Input should be a valid integer")),
    }
}

fn int_text(s: &str) -> Option<i64> {
    let s = match s.split_once('.') {
        Some((whole, zeros)) if !zeros.is_empty() && zeros.bytes().all(|b| b == b'0') => whole,
        Some(_) => return None,
        None => s,
    };
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let ok = !digits.is_empty()
        && !digits.starts_with('_')
        && !digits.ends_with('_')
        && !digits.contains("__")
        && digits.bytes().all(|c| c.is_ascii_digit() || c == b'_');
    if !ok {
        return None;
    }
    digits
        .replace('_', "")
        .parse::<i64>()
        .ok()
        .map(|n| sign * n)
}

/// `bool`: `true`/`false`, the integers 0 and 1, or one of pydantic's words.
pub fn boolean(v: &Value, field: &str) -> Result<bool, Refusal> {
    const UNREADABLE: &str = "Input should be a valid boolean, unable to interpret input";
    let bad = || Refusal::new(field, "Input should be a valid boolean");
    let unreadable = || Refusal::new(field, UNREADABLE);
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(0), _) => Ok(false),
            (Some(1), _) => Ok(true),
            (Some(_), _) => Err(unreadable()),
            (None, Some(0.0)) => Ok(false),
            (None, Some(1.0)) => Ok(true),
            _ => Err(bad()),
        },
        // lowercased, not stripped: " yes" is refused
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "1" | "on" | "t" | "true" | "y" | "yes" => Ok(true),
            "0" | "off" | "f" | "false" | "n" | "no" => Ok(false),
            _ => Err(unreadable()),
        },
        _ => Err(bad()),
    }
}

/// `bool = <default>`: absent is the default (`null` is refused, as pydantic does).
pub fn opt_boolean(v: Option<&Value>, field: &str, default: bool) -> Result<bool, Refusal> {
    v.map_or(Ok(default), |v| boolean(v, field))
}
