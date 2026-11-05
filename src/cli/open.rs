//! `inbox open [--question Qn | --group G | --pane P | --session S]
//! [--no-hold]` (Python `cli._command`'s `open`): asks herdr to open the
//! popup, on one target if named (a focus request the popup takes on start,
//! [`crate::deeplink`]).

use super::Io;
use super::args::parse::{Args, Parsed, top_error};
use super::args::pyrepr::repr;
use super::args::question_ref;
use crate::deeplink::lock::popup_is_running;
use crate::deeplink::request::{unix_now, write_request};
use crate::deeplink::{KINDS, NONCE_ENV, valid};

/// What `open` says when the popup is open already.
pub const ALREADY_OPEN: &str = "the inbox popup is already open; close it and run this again";

/// Opens the popup: 0, 1 when herdr refused, 2 for a malformed target (as
/// argparse's `parser.error`), 3 when a popup is already open (it would
/// never take the request).
pub fn open(args: &Args, io: &mut Io<'_>) -> Result<i32, Parsed> {
    let mut env = Vec::new();
    for kind in KINDS {
        let Some(value) = args.value(&format!("--{kind}")).filter(|v| !v.is_empty()) else {
            continue;
        };
        if !valid(kind, value) {
            let expected = match kind {
                "question" => "Qn or n",
                "group" => "a group number",
                _ => "an id",
            };
            return Err(top_error(&format!(
                "--{kind} {}: expected {expected}",
                repr(value)
            )));
        }
        if popup_is_running(io.state_dir) {
            let _ = writeln!(io.err, "{ALREADY_OPEN}");
            return Ok(3);
        }
        let stored = if kind == "question" {
            question_ref(value)
        } else {
            value.to_string()
        };
        let hold = !args.has("--no-hold");
        // Python raises here (an unwritable state directory): a traceback, exit 1
        let Some(nonce) = write_request(io.state_dir, kind, &stored, hold, unix_now()) else {
            let _ = writeln!(io.err, "inbox: could not write the open request");
            return Ok(1);
        };
        env.push((NONCE_ENV.to_string(), nonce));
    }
    if let Err(err) = io.herdr.open_popup(&env) {
        let _ = writeln!(io.err, "could not open the inbox popup: {err}");
        return Ok(1);
    }
    Ok(0)
}
