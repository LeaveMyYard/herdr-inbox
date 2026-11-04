//! `ask_operator`'s arguments as pydantic hands them to the Python function:
//! each option a string or an `Option` with only its own keys, each item of
//! `questions` a `QuestionItem` with only its own keys.

use serde_json::{Map, Value};

use super::lax::{MISSING, Refusal, boolean, opt_boolean, opt_string, string};

/// One question of a call: `question` itself, or an item of `questions`.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The question's text.
    pub question: String,
    /// Its chip, if any.
    pub header: Option<String>,
    /// Its choices (strings or `{"label", ...}` objects), if any.
    pub options: Option<Vec<Value>>,
    /// Several choices may be picked together.
    pub multi_select: bool,
}

/// `ask_operator`'s arguments.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ask {
    pub question: Option<String>,
    pub options: Option<Vec<Value>>,
    pub context: Option<String>,
    pub asked_by: Option<String>,
    pub header: Option<String>,
    pub multi_select: bool,
    pub questions: Option<Vec<Item>>,
    pub title: Option<String>,
}

/// An `Option` TypedDict: `label` required, `description` and `preview`
/// optional, every other key dropped.
fn option(v: &Map<String, Value>, at: &str) -> Result<Value, Refusal> {
    let mut out = Map::new();
    let label = v
        .get("label")
        .ok_or_else(|| Refusal::new(format!("{at}.label"), MISSING))?;
    out.insert(
        "label".into(),
        Value::String(string(label, &format!("{at}.label"))?),
    );
    for key in ["description", "preview"] {
        if let Some(x) = v.get(key) {
            out.insert(
                key.into(),
                Value::String(string(x, &format!("{at}.{key}"))?),
            );
        }
    }
    Ok(Value::Object(out))
}

/// `list[str | Option]`.
fn options(v: &Value, at: &str) -> Result<Vec<Value>, Refusal> {
    let items = v
        .as_array()
        .ok_or_else(|| Refusal::new(at, "Input should be a valid list"))?;
    items
        .iter()
        .enumerate()
        .map(|(i, o)| match o {
            Value::String(_) => Ok(o.clone()),
            Value::Object(m) => option(m, &format!("{at}.{i}")),
            _ => Err(Refusal::new(
                format!("{at}.{i}"),
                "Input should be a valid string or an Option object",
            )),
        })
        .collect()
}

/// `list[str | Option] | None`.
fn opt_options(v: Option<&Value>, at: &str) -> Result<Option<Vec<Value>>, Refusal> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => options(v, at).map(Some),
    }
}

/// A `QuestionItem` TypedDict: `question` required, the rest optional.
fn item(v: &Value, at: &str) -> Result<Item, Refusal> {
    let m = v
        .as_object()
        .ok_or_else(|| Refusal::new(at, "Input should be a valid dictionary"))?;
    let question = m
        .get("question")
        .ok_or_else(|| Refusal::new(format!("{at}.question"), MISSING))?;
    let header = match m.get("header") {
        Some(h) => Some(string(h, &format!("{at}.header"))?),
        None => None,
    };
    let options = match m.get("options") {
        Some(o) => Some(options(o, &format!("{at}.options"))?),
        None => None,
    };
    let multi_select = match m.get("multi_select") {
        Some(b) => boolean(b, &format!("{at}.multi_select"))?,
        None => false,
    };
    Ok(Item {
        question: string(question, &format!("{at}.question"))?,
        header,
        options,
        multi_select,
    })
}

/// `list[QuestionItem] | None`.
fn items(v: Option<&Value>) -> Result<Option<Vec<Item>>, Refusal> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(xs)) => xs
            .iter()
            .enumerate()
            .map(|(i, x)| item(x, &format!("questions.{i}")))
            .collect::<Result<_, _>>()
            .map(Some),
        Some(_) => Err(Refusal::new("questions", "Input should be a valid list")),
    }
}

/// The call's arguments, or the first one pydantic would refuse.
pub fn parse(args: &Map<String, Value>) -> Result<Ask, Refusal> {
    Ok(Ask {
        question: opt_string(args.get("question"), "question")?,
        options: opt_options(args.get("options"), "options")?,
        context: opt_string(args.get("context"), "context")?,
        asked_by: opt_string(args.get("asked_by"), "asked_by")?,
        header: opt_string(args.get("header"), "header")?,
        multi_select: opt_boolean(args.get("multi_select"), "multi_select", false)?,
        questions: items(args.get("questions"))?,
        title: opt_string(args.get("title"), "title")?,
    })
}
