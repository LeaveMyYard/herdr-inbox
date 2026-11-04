//! A `tools/call`'s arguments checked against the tool's signature, as the
//! Python server's pydantic model checks them before the tool runs: unknown
//! keys are ignored, absent ones take their defaults, values are read in
//! pydantic's lax mode ([`lax`]). A refused call never reaches the tool.

pub mod ask;
pub mod lax;

use serde_json::{Map, Value};

pub use ask::{Ask, Item};
pub use lax::Refusal;
use lax::{MISSING, int, opt_boolean, opt_string, string};

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

fn required<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a Value, Refusal> {
    args.get(key).ok_or_else(|| Refusal::new(key, MISSING))
}

/// `tool`'s call with `args`, or the first argument pydantic would refuse.
pub fn parse(tool: Name, args: &Map<String, Value>) -> Result<Call, Refusal> {
    Ok(match tool {
        Name::AskOperator => Call::AskOperator(ask::parse(args)?),
        Name::AlertOperator => Call::AlertOperator {
            title: string(required(args, "title")?, "title")?,
            message: opt_string(args.get("message"), "message")?,
            severity: match args.get("severity") {
                None => "warning".into(),
                Some(v) => string(v, "severity")?,
            },
            asked_by: opt_string(args.get("asked_by"), "asked_by")?,
        },
        Name::WithdrawAlert => Call::WithdrawAlert {
            id: int(required(args, "id")?, "id")?,
        },
        Name::CheckAnswers => Call::CheckAnswers,
        Name::ListMyQuestions => Call::ListMyQuestions {
            include_closed: opt_boolean(args.get("include_closed"), "include_closed", false)?,
        },
        Name::WithdrawQuestion => Call::WithdrawQuestion {
            id: int(required(args, "id")?, "id")?,
            reason: string(required(args, "reason")?, "reason")?,
        },
    })
}

/// The tool error a refused call gets: pydantic's first lines (the field and
/// the reason), without its error code and link.
pub fn refused(tool: Name, why: &Refusal) -> String {
    let name = tool.as_str();
    format!(
        "Error executing tool {name}: 1 validation error for {name}Arguments\n{}\n  {}",
        why.field, why.why
    )
}

#[cfg(test)]
mod tests;
