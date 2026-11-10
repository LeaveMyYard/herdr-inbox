//! A verb's command line. Anything wrong here is exit 2, with no I/O.

use serde_json::Value;

use super::detail::{self, MAX_DETAIL};

/// The largest id a verb takes: `Number.MAX_SAFE_INTEGER`, as the feed publishes ids.
const MAX_ID: i64 = 9_007_199_254_740_991;

/// A verb that writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// `answer`.
    Answer,
    /// `draft`.
    Draft,
    /// `dismiss`.
    Dismiss,
    /// `retry`.
    Retry,
    /// `stop-showing`.
    StopShowing,
    /// `ack`.
    Ack,
}

impl Verb {
    /// Its name.
    pub fn name(self) -> &'static str {
        match self {
            Verb::Answer => "answer",
            Verb::Draft => "draft",
            Verb::Dismiss => "dismiss",
            Verb::Retry => "retry",
            Verb::StopShowing => "stop-showing",
            Verb::Ack => "ack",
        }
    }

    fn named(name: &str) -> Option<Verb> {
        [
            Verb::Answer,
            Verb::Draft,
            Verb::Dismiss,
            Verb::Retry,
            Verb::StopShowing,
            Verb::Ack,
        ]
        .into_iter()
        .find(|v| v.name() == name)
    }
}

/// What `--entry` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// `group:<G>`.
    Group(i64),
    /// `failed:<Q>`.
    Failed(i64),
    /// `alert:<A>`.
    Alert(i64),
    /// A remote machine's row.
    Remote,
}

/// A verb to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// The verb.
    pub verb: Verb,
    /// `--entry`, as given.
    pub key: String,
    /// What it names.
    pub entry: Entry,
    /// `--token`.
    pub token: String,
    /// Every `--question`, once each, in the order given.
    pub questions: Vec<i64>,
    /// `--detail`, a well-formed Detail.
    pub detail: Option<Value>,
}

impl Request {
    /// Why this row takes no such verb, before any database is read.
    pub fn not_for_this_row(&self) -> Option<&'static str> {
        match (self.entry, self.verb) {
            (Entry::Remote, _) => Some("Answer this in the herdr popup"),
            (Entry::Group(_), Verb::Answer | Verb::Draft | Verb::Dismiss)
            | (Entry::Failed(_), Verb::Retry | Verb::StopShowing)
            | (Entry::Alert(_), Verb::Ack) => None,
            _ => Some("This row does not take that action"),
        }
    }
}

/// What a command line asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    /// `app --help`.
    Help,
    /// Bad arguments, and why.
    Usage(String),
    /// `show`.
    Show {
        /// The question.
        question: i64,
        /// `--json`.
        json: bool,
    },
    /// A verb.
    Run(Request),
}

/// A canonical id: decimal in [1, 2^53 − 1], no sign, prefix or leading zero.
pub fn canonical_id(word: &str) -> Option<i64> {
    let digits = word.bytes().all(|b| b.is_ascii_digit());
    if !digits || word.is_empty() || word.starts_with('0') || word.len() > 16 {
        return None;
    }
    word.parse().ok().filter(|id| *id <= MAX_ID)
}

fn entry(word: &str) -> Option<Entry> {
    let (kind, rest) = word.split_once(':')?;
    match kind {
        "group" => canonical_id(rest).map(Entry::Group),
        "failed" => canonical_id(rest).map(Entry::Failed),
        "alert" => canonical_id(rest).map(Entry::Alert),
        "remote" | "routbox" | "rproblem" | "ralert" if !rest.is_empty() => Some(Entry::Remote),
        _ => None,
    }
}

/// The words of a command, each option in its place.
#[derive(Debug, Default)]
struct Words {
    entry: Option<String>,
    token: Option<String>,
    questions: Vec<i64>,
    detail: Option<String>,
    clear: bool,
    json: bool,
}

fn words(argv: &[String]) -> Result<Words, String> {
    let mut w = Words::default();
    let mut rest = argv.iter();
    while let Some(word) = rest.next() {
        let mut value = || rest.next().ok_or_else(|| format!("{word} needs a value"));
        match word.as_str() {
            "--entry" if w.entry.is_none() => w.entry = Some(value()?.clone()),
            "--token" if w.token.is_none() => w.token = Some(value()?.clone()),
            "--detail" if w.detail.is_none() => w.detail = Some(value()?.clone()),
            "--question" => {
                let id = canonical_id(value()?).ok_or("--question takes a canonical id")?;
                if !w.questions.contains(&id) {
                    w.questions.push(id);
                }
            }
            "--clear" if !w.clear => w.clear = true,
            "--json" if !w.json => w.json = true,
            _ => return Err("unexpected argument".to_string()), // never its words: it may be an answer
        }
    }
    Ok(w)
}

/// Parses the words after `app`.
pub fn parse(argv: &[String]) -> Parsed {
    let Some((first, rest)) = argv.split_first() else {
        return Parsed::Usage("no verb".to_string());
    };
    if first == "--help" && rest.is_empty() {
        return Parsed::Help;
    }
    let parsed = words(rest).and_then(|w| {
        if first == "show" {
            show(&w)
        } else {
            let verb = Verb::named(first).ok_or("no such verb")?;
            request(verb, w).map(Parsed::Run)
        }
    });
    parsed.unwrap_or_else(Parsed::Usage)
}

fn show(w: &Words) -> Result<Parsed, String> {
    match (&w.questions[..], &w.entry, &w.token, &w.detail, w.clear) {
        ([question], None, None, None, false) => Ok(Parsed::Show {
            question: *question,
            json: w.json,
        }),
        _ => Err("show takes --question Q [--json]".to_string()),
    }
}

fn request(verb: Verb, w: Words) -> Result<Request, String> {
    if !w.json {
        return Err("--json is required".to_string());
    }
    let key = w.entry.ok_or("--entry is required")?;
    let token = w.token.ok_or("--token is required")?;
    let kind = entry(&key).ok_or("--entry is malformed")?;
    let (asked, has_detail) = (w.questions.len(), w.detail.is_some());
    let fits = match verb {
        Verb::Answer => asked >= 1 && !w.clear && (!has_detail || asked == 1),
        Verb::Draft => asked == 1 && has_detail != w.clear,
        Verb::Dismiss => asked <= 1 && !w.clear && !has_detail,
        Verb::Retry | Verb::StopShowing | Verb::Ack => asked == 0 && !w.clear && !has_detail,
    };
    if !fits {
        return Err(format!("{} does not take those options", verb.name()));
    }
    let detail = w.detail.as_deref().map(parse_detail).transpose()?;
    Ok(Request {
        verb,
        key,
        entry: kind,
        token,
        questions: w.questions,
        detail,
    })
}

fn parse_detail(text: &str) -> Result<Value, String> {
    if text.len() > MAX_DETAIL {
        return Err("--detail is over 64 KiB".to_string());
    }
    let value = serde_json::from_str(text).map_err(|_| "--detail is no JSON")?;
    detail::well_formed(&value)
        .then_some(value)
        .ok_or_else(|| "--detail is no Detail".to_string())
}

#[cfg(test)]
mod tests;
