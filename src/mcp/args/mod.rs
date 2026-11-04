//! A `tools/call`'s arguments checked against the tool's signature, as the
//! Python server's pydantic model checks them before the tool runs: unknown
//! keys are ignored, absent ones take their defaults, values are read in
//! pydantic's lax mode ([`lax`]), after FastMCP's JSON pre-pass
//! ([`preparse`]). A refused call never reaches the tool; its error lists
//! every refusal, as pydantic does.

pub mod ask;
pub mod lax;
pub mod preparse;

use serde_json::{Map, Value};

pub use ask::{Ask, Item};
pub use lax::Refusal;
use lax::{MISSING, Refusals, int, opt_boolean, opt_string, string};

use super::tools::Name;

/// One call of one tool, with its arguments.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    AskOperator(Ask),
    AlertOperator {
        title: String,
        message: Option<String>,
        severity: String,
        asked_by: Option<String>,
    },
    WithdrawAlert {
        id: i64,
    },
    CheckAnswers,
    ListMyQuestions {
        include_closed: bool,
    },
    WithdrawQuestion {
        id: i64,
        reason: String,
    },
}

fn required<'a>(args: &'a Map<String, Value>, key: &str, errs: &mut Refusals) -> Option<&'a Value> {
    let v = args.get(key);
    if v.is_none() {
        errs.push(Refusal::new(key, MISSING));
    }
    v
}

/// `tool`'s call with `args`, or every refusal pydantic would give.
pub fn parse(tool: Name, args: &Map<String, Value>) -> Result<Call, Vec<Refusal>> {
    let args = &preparse::preparse(tool, args);
    let mut e = Refusals::default();
    let e = &mut e;
    let call = match tool {
        Name::AskOperator => return ask::parse(args).map(Call::AskOperator),
        Name::AlertOperator => {
            let title = required(args, "title", e).and_then(|v| e.take(string(v, "title")));
            let message = e.take(opt_string(args.get("message"), "message")).flatten();
            let severity = match args.get("severity") {
                None => Some("warning".to_owned()),
                Some(v) => e.take(string(v, "severity")),
            };
            let asked_by = e
                .take(opt_string(args.get("asked_by"), "asked_by"))
                .flatten();
            title
                .zip(severity)
                .map(|(title, severity)| Call::AlertOperator {
                    title,
                    message,
                    severity,
                    asked_by,
                })
        }
        Name::WithdrawAlert => required(args, "id", e)
            .and_then(|v| e.take(int(v, "id")))
            .map(|id| Call::WithdrawAlert { id }),
        Name::CheckAnswers => Some(Call::CheckAnswers),
        Name::ListMyQuestions => e
            .take(opt_boolean(
                args.get("include_closed"),
                "include_closed",
                false,
            ))
            .map(|include_closed| Call::ListMyQuestions { include_closed }),
        Name::WithdrawQuestion => {
            let id = required(args, "id", e).and_then(|v| e.take(int(v, "id")));
            let reason = required(args, "reason", e).and_then(|v| e.take(string(v, "reason")));
            id.zip(reason)
                .map(|(id, reason)| Call::WithdrawQuestion { id, reason })
        }
    };
    // a value is missing only where a refusal was recorded
    match (call, std::mem::take(e).done(())) {
        (Some(call), Ok(())) => Ok(call),
        (_, Err(why)) => Err(why),
        (None, Ok(())) => Err(Vec::new()),
    }
}

/// The tool error a refused call gets: pydantic's text, each refused field
/// and its reason, without pydantic's error codes, inputs and links.
pub fn refused(tool: Name, why: &[Refusal]) -> String {
    let name = tool.as_str();
    let n = why.len();
    let plural = if n == 1 { "" } else { "s" };
    let mut out =
        format!("Error executing tool {name}: {n} validation error{plural} for {name}Arguments");
    for r in why {
        out.push_str(&format!("\n{}\n  {}", r.field, r.why));
    }
    out
}

#[cfg(test)]
mod tests;
