//! The hook's stdout (Python `json.dumps` with its defaults: `", "` and
//! `": "` separators, every non-ASCII character escaped), byte for byte.

use crate::pyjson::string as json_string;

/// `{"hookSpecificOutput": {"hookEventName": …, "additionalContext": …}}`.
pub fn hook_output(event_name: &str, context: &str) -> String {
    format!(
        "{{\"hookSpecificOutput\": {{\"hookEventName\": {}, \"additionalContext\": {}}}}}",
        json_string(event_name),
        json_string(context)
    )
}

#[cfg(test)]
mod tests;
