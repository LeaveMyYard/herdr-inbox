//! A payload nested deeper than serde_json's default limit (128 levels):
//! Python's `json.loads` reads it, and PostToolBatch payloads carry tool
//! inputs and responses (MCP `structuredContent`) as deep as they come.

use std::thread;

use serde::Deserialize;
use serde_json::{Map, Value};

/// The fields a hook reads; the rest of a deep payload is dropped unread.
const FIELDS: [&str; 4] = ["session_id", "transcript_path", "source", "agent_id"];

/// The stack for parsing without a depth limit: only the pages used are
/// committed.
const STACK: usize = 512 << 20;

/// Deeper than this is not read at all, as Python's parser gives up (about 100k
/// levels); it also bounds the stack the parse below needs.
pub const MAX_DEPTH: usize = 100_000;

/// The deepest nesting of arrays and objects in `raw`, strings skipped.
/// A count, never a parse: no recursion, whatever the input.
pub fn depth(raw: &str) -> usize {
    let (mut now, mut max, mut in_string, mut escaped) = (0usize, 0usize, false, false);
    for b in raw.bytes() {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                now += 1;
                max = max.max(now);
            }
            b']' | b'}' => now = now.saturating_sub(1),
            _ => {}
        }
    }
    max
}

/// serde_json gave up on the depth, not on the syntax.
pub fn too_deep(e: &serde_json::Error) -> bool {
    e.to_string().starts_with("recursion limit exceeded")
}

/// The top-level fields a hook reads of a deep JSON object; `None` when it is
/// not a JSON object. Parsed with no depth limit on a thread with a large
/// stack (parsing and dropping a deep value both recurse).
pub fn top_level(raw: &str) -> Option<Map<String, Value>> {
    if depth(raw) > MAX_DEPTH {
        return None; // Python gives up too (RecursionError: the hook exits 0, prints nothing)
    }
    let raw = raw.to_owned();
    let parse = move || {
        let mut de = serde_json::Deserializer::from_str(&raw);
        de.disable_recursion_limit();
        let Ok(Value::Object(mut all)) = Value::deserialize(&mut de) else {
            return None;
        };
        de.end().ok()?;
        // only shallow values leave this thread: strings as they are, anything else reduced
        // to its truthiness (all a hook reads of it); the deep rest is dropped here
        let kept: Map<String, Value> = FIELDS
            .iter()
            .filter_map(|k| all.remove(*k).map(|v| (k.to_string(), shallow(v))))
            .collect();
        Some(kept)
    };
    thread::Builder::new()
        .stack_size(STACK)
        .spawn(parse)
        .ok()?
        .join()
        .ok()?
}

/// `v` itself when it is a string, else a boolean of its Python truthiness.
fn shallow(v: Value) -> Value {
    match v {
        Value::String(_) => v,
        Value::Array(a) => Value::Bool(!a.is_empty()),
        Value::Object(o) => Value::Bool(!o.is_empty()),
        other => Value::Bool(super::truthy(&other)),
    }
}
