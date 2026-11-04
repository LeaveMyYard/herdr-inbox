//! FastMCP's JSON pre-pass (`func_metadata.pre_parse_json`), which runs before
//! pydantic sees a call: for every argument not annotated plain `str`, a
//! string that parses as JSON to a list, an object or `null` is replaced by
//! that value (clients send `["a", "b"]` as a JSON string). A string that
//! parses to a string, a number or a boolean (Python's `bool` is an `int`),
//! or does not parse, is left as it is. Only the top-level arguments are read.

use serde_json::{Map, Value};

use crate::mcp::tools::Name;
use crate::pyjson::loads;

/// The arguments of `tool` annotated plain `str`: never pre-parsed.
fn plain_str(tool: Name) -> &'static [&'static str] {
    match tool {
        Name::AlertOperator => &["title", "severity"],
        Name::WithdrawQuestion => &["reason"],
        _ => &[],
    }
}

/// The arguments of `tool` (every other key is ignored by pydantic anyway).
fn fields(tool: Name) -> &'static [&'static str] {
    match tool {
        Name::AskOperator => &[
            "question",
            "options",
            "context",
            "asked_by",
            "header",
            "multi_select",
            "questions",
            "title",
        ],
        Name::AlertOperator => &["title", "message", "severity", "asked_by"],
        Name::WithdrawAlert => &["id"],
        Name::CheckAnswers => &[],
        Name::ListMyQuestions => &["include_closed"],
        Name::WithdrawQuestion => &["id", "reason"],
    }
}

/// `args` after the pre-pass.
pub fn preparse(tool: Name, args: &Map<String, Value>) -> Map<String, Value> {
    let mut out = args.clone();
    for (key, value) in out.iter_mut() {
        if !fields(tool).contains(&key.as_str()) || plain_str(tool).contains(&key.as_str()) {
            continue;
        }
        let Value::String(text) = value else { continue };
        if let Ok(parsed @ (Value::Array(_) | Value::Object(_) | Value::Null)) = loads(text) {
            *value = parsed;
        }
    }
    out
}
