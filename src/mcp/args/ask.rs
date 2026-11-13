//! `ask_operator`'s arguments as pydantic hands them to the Python function:
//! each option a string or an `Option` with only its own keys, each item of
//! `questions` a `QuestionItem` with only its own keys. Every refusal is
//! collected, in pydantic's order (declaration order, then position).

use serde_json::{Map, Value};

use super::lax::{MISSING, Refusal, Refusals, boolean, opt_boolean, opt_string, string};

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
    /// The call's `tool_use_id`, from its `_meta` (never from the arguments):
    /// set by the server after [`parse`], already [`crate::store::tool_use::checked`].
    pub tool_use_id: Option<String>,
}

/// `str | Option` (pydantic's smart union): a string stays a string; an
/// `Option` keeps `label` (required), `description` and `preview`. A value
/// that is neither fails both branches, the `str` branch reported first.
fn option(v: &Value, at: &str, errs: &mut Refusals) -> Option<Value> {
    if v.is_string() {
        return Some(v.clone());
    }
    let str_branch = Refusal::new(format!("{at}.str"), "Input should be a valid string");
    let at = format!("{at}.Option");
    let Some(m) = v.as_object() else {
        errs.push(str_branch);
        errs.push(Refusal::new(at, "Input should be a valid dictionary"));
        return None;
    };
    let mut mine = Refusals::default();
    let mut out = Map::new();
    match m.get("label") {
        None => mine.push(Refusal::new(format!("{at}.label"), MISSING)),
        Some(l) => {
            if let Some(l) = mine.take(string(l, &format!("{at}.label"))) {
                out.insert("label".into(), Value::String(l));
            }
        }
    }
    for key in ["description", "preview"] {
        if let Some(x) = m.get(key)
            && let Some(x) = mine.take(string(x, &format!("{at}.{key}")))
        {
            out.insert(key.into(), Value::String(x));
        }
    }
    if mine.is_empty() {
        return Some(Value::Object(out));
    }
    errs.push(str_branch);
    errs.extend(mine);
    None
}

/// `list[str | Option]`.
fn options(v: &Value, at: &str, errs: &mut Refusals) -> Option<Vec<Value>> {
    let Some(items) = v.as_array() else {
        errs.push(Refusal::new(at, "Input should be a valid list"));
        return None;
    };
    let before = errs.len();
    let out: Vec<Option<Value>> = items
        .iter()
        .enumerate()
        .map(|(i, o)| option(o, &format!("{at}.{i}"), errs))
        .collect();
    (errs.len() == before).then(|| out.into_iter().flatten().collect())
}

/// `list[str | Option] | None`.
fn opt_options(v: Option<&Value>, at: &str, errs: &mut Refusals) -> Option<Vec<Value>> {
    match v {
        None | Some(Value::Null) => None,
        Some(v) => options(v, at, errs),
    }
}

/// A `QuestionItem` TypedDict: `question` required, the rest optional.
fn item(v: &Value, at: &str, errs: &mut Refusals) -> Option<Item> {
    let Some(m) = v.as_object() else {
        errs.push(Refusal::new(at, "Input should be a valid dictionary"));
        return None;
    };
    let before = errs.len();
    let question = match m.get("question") {
        None => {
            errs.push(Refusal::new(format!("{at}.question"), MISSING));
            None
        }
        Some(q) => errs.take(string(q, &format!("{at}.question"))),
    };
    let header = m
        .get("header")
        .and_then(|h| errs.take(string(h, &format!("{at}.header"))));
    let options = m
        .get("options")
        .and_then(|o| options(o, &format!("{at}.options"), errs));
    let multi_select = m
        .get("multi_select")
        .and_then(|b| errs.take(boolean(b, &format!("{at}.multi_select"))))
        .unwrap_or(false);
    if errs.len() != before {
        return None;
    }
    Some(Item {
        question: question?,
        header,
        options,
        multi_select,
    })
}

/// `list[QuestionItem] | None`.
fn items(v: Option<&Value>, errs: &mut Refusals) -> Option<Vec<Item>> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::Array(xs)) => {
            let before = errs.len();
            let out: Vec<Option<Item>> = xs
                .iter()
                .enumerate()
                .map(|(i, x)| item(x, &format!("questions.{i}"), errs))
                .collect();
            (errs.len() == before).then(|| out.into_iter().flatten().collect())
        }
        Some(_) => {
            errs.push(Refusal::new("questions", "Input should be a valid list"));
            None
        }
    }
}

/// The call's arguments, or every refusal pydantic would give, in its order.
pub fn parse(args: &Map<String, Value>) -> Result<Ask, Vec<Refusal>> {
    let mut errs = Refusals::default();
    let ask = Ask {
        question: errs
            .take(opt_string(args.get("question"), "question"))
            .flatten(),
        options: opt_options(args.get("options"), "options", &mut errs),
        context: errs
            .take(opt_string(args.get("context"), "context"))
            .flatten(),
        asked_by: errs
            .take(opt_string(args.get("asked_by"), "asked_by"))
            .flatten(),
        header: errs
            .take(opt_string(args.get("header"), "header"))
            .flatten(),
        multi_select: errs
            .take(opt_boolean(args.get("multi_select"), "multi_select", false))
            .unwrap_or(false),
        questions: items(args.get("questions"), &mut errs),
        title: errs.take(opt_string(args.get("title"), "title")).flatten(),
        tool_use_id: None,
    };
    errs.done(ask)
}
