//! Step timings for latency investigations (Python `trace.py`):
//! `INBOX_TRACE=<file>`, off by default. Each step appends one JSON line
//! `{"t": <unix>, "pid", "thread", "step", "ms"}` to the file, from any
//! process (popup, MCP server, worker, CLI). Off, a step costs one environment
//! lookup. The file is the investigator's: a failed write is ignored, and the
//! variable is read at each step, as Python reads it.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::pyjson::dumps;

/// The variable naming the trace file.
pub const TRACE_VAR: &str = "INBOX_TRACE";

/// The trace file, if tracing is on (an empty value is off, as in Python).
fn target() -> Option<OsString> {
    std::env::var_os(TRACE_VAR).filter(|p| !p.is_empty())
}

/// Tracing is on (Python `trace.enabled`).
pub fn enabled() -> bool {
    target().is_some()
}

/// `v` to `places` decimals (Python `round`).
fn rounded(v: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (v * scale).round() / scale
}

/// The JSON line of a step taken now in this thread.
fn line(step: &str, ms: f64, extra: &[(&str, Value)]) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let thread = std::thread::current();
    let mut out = Map::new();
    out.insert("t".into(), json!(rounded(now, 4)));
    out.insert("pid".into(), json!(std::process::id()));
    out.insert("thread".into(), json!(thread.name().unwrap_or("unnamed")));
    out.insert("step".into(), json!(step));
    out.insert("ms".into(), json!(rounded(ms, 2)));
    for (key, value) in extra {
        out.insert((*key).to_string(), value.clone());
    }
    dumps(&Value::Object(out))
}

/// Appends `step` to `path`, one write per line. Tracing never fails its caller.
fn append(path: &Path, step: &str, ms: f64, extra: &[(&str, Value)]) {
    let text = line(step, ms, extra) + "\n";
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(text.as_bytes());
    }
}

/// Records a step that took `ms` (Python `trace.record`).
pub fn record(step: &str, ms: f64, extra: &[(&str, Value)]) {
    if let Some(path) = target() {
        append(Path::new(&path), step, ms, extra);
    }
}

/// A step being timed: recorded when dropped, however its scope ends.
#[must_use = "a step ends when this is dropped: bind it to a named variable"]
pub struct Step {
    open: Option<Open>,
}

struct Open {
    name: String,
    extra: Vec<(&'static str, Value)>,
    started: Instant,
}

/// Starts timing `name` (Python `with trace.step(name)`).
pub fn step(name: &str) -> Step {
    step_with(name, Vec::new())
}

/// [`step`] with more fields on its line (Python `trace.step(name, **extra)`).
pub fn step_with(name: &str, extra: Vec<(&'static str, Value)>) -> Step {
    Step {
        open: enabled().then(|| Open {
            name: name.to_string(),
            extra,
            started: Instant::now(),
        }),
    }
}

/// `work` as the step `name`.
pub fn timed<T>(name: &str, work: impl FnOnce() -> T) -> T {
    let _step = step(name);
    work()
}

/// `work`, a call to herdr's socket method `method`, as the step of the CLI
/// call it stands for in Python (`pane.report_metadata`: `herdr pane
/// report-metadata`).
pub fn herdr<T>(method: &str, work: impl FnOnce() -> T) -> T {
    timed(&step_name(method), work)
}

fn step_name(method: &str) -> String {
    format!("herdr {}", method.replace('.', " ").replace('_', "-"))
}

impl Drop for Step {
    fn drop(&mut self) {
        if let Some(open) = self.open.take() {
            let ms = open.started.elapsed().as_secs_f64() * 1000.0;
            record(&open.name, ms, &open.extra);
        }
    }
}

#[cfg(test)]
mod tests;
