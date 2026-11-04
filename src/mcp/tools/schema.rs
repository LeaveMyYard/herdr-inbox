//! The tools' JSON schemas as pydantic made them of the Python signatures
//! (`server.py`), key for key and in pydantic's key order: `tests/mcp_goldens.rs`
//! compares their bytes with the Python server's `tools/list`.

use serde_json::{Value, json};

/// `str | None = None`.
fn opt_string(title: &str) -> Value {
    json!({"anyOf": [{"type": "string"}, {"type": "null"}], "default": null, "title": title})
}

/// A required field of type `ty`.
fn required(title: &str, ty: &str) -> Value {
    json!({"title": title, "type": ty})
}

/// A field of type `ty` with a default.
fn defaulted(default: Value, title: &str, ty: &str) -> Value {
    json!({"default": default, "title": title, "type": ty})
}

/// `list[str | Option]`, as an item's `options` and the call's.
fn option_items() -> Value {
    json!({"anyOf": [{"type": "string"}, {"$ref": "#/$defs/Option"}]})
}

/// The `$defs` of `ask_operator`: the `Option` and `QuestionItem` TypedDicts.
fn ask_defs() -> Value {
    json!({
        "Option": {
            "properties": {
                "label": required("Label", "string"),
                "description": required("Description", "string"),
                "preview": required("Preview", "string")
            },
            "required": ["label"],
            "title": "Option",
            "type": "object"
        },
        "QuestionItem": {
            "properties": {
                "question": required("Question", "string"),
                "header": required("Header", "string"),
                "options": {"items": option_items(), "title": "Options", "type": "array"},
                "multi_select": required("Multi Select", "boolean")
            },
            "required": ["question"],
            "title": "QuestionItem",
            "type": "object"
        }
    })
}

/// `ask_operator`'s arguments.
pub fn ask_operator() -> Value {
    let list_of = |items: Value, title: &str| {
        json!({
            "anyOf": [{"items": items, "type": "array"}, {"type": "null"}],
            "default": null,
            "title": title
        })
    };
    json!({
        "properties": {
            "question": opt_string("Question"),
            "options": list_of(option_items(), "Options"),
            "context": opt_string("Context"),
            "asked_by": opt_string("Asked By"),
            "header": opt_string("Header"),
            "multi_select": defaulted(json!(false), "Multi Select", "boolean"),
            "questions": list_of(json!({"$ref": "#/$defs/QuestionItem"}), "Questions"),
            "title": opt_string("Title")
        },
        "type": "object",
        "$defs": ask_defs(),
        "title": "ask_operatorArguments"
    })
}

/// `alert_operator`'s arguments.
pub fn alert_operator() -> Value {
    json!({
        "properties": {
            "title": required("Title", "string"),
            "message": opt_string("Message"),
            "severity": defaulted(json!("warning"), "Severity", "string"),
            "asked_by": opt_string("Asked By")
        },
        "required": ["title"],
        "type": "object",
        "title": "alert_operatorArguments"
    })
}

/// The arguments of a tool that takes only `id` (and `reason`, if `with_reason`).
pub fn by_id(tool: &str, with_reason: bool) -> Value {
    let mut props = serde_json::Map::new();
    props.insert("id".into(), required("Id", "integer"));
    let mut req = vec![json!("id")];
    if with_reason {
        props.insert("reason".into(), required("Reason", "string"));
        req.push(json!("reason"));
    }
    json!({
        "properties": props,
        "required": req,
        "type": "object",
        "title": format!("{tool}Arguments")
    })
}

/// `check_answers`'s arguments: none.
pub fn check_answers() -> Value {
    json!({"properties": {}, "type": "object", "title": "check_answersArguments"})
}

/// `list_my_questions`'s arguments.
pub fn list_my_questions() -> Value {
    json!({
        "properties": {
            "include_closed": defaulted(json!(false), "Include Closed", "boolean")
        },
        "type": "object",
        "title": "list_my_questionsArguments"
    })
}

/// Every tool's result: a dict.
pub fn output(tool: &str) -> Value {
    json!({"type": "object", "additionalProperties": true, "title": format!("{tool}DictOutput")})
}
