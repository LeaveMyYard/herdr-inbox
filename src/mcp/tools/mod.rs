//! The six tools as `tools/list` lists them: their names in the Python
//! server's order, their descriptions ([`text`]) and schemas ([`schema`]).

pub mod schema;
pub mod text;

use std::sync::Arc;

use rmcp::model::{JsonObject, Tool};
use serde_json::Value;

/// A tool of this server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Name {
    AskOperator,
    AlertOperator,
    WithdrawAlert,
    CheckAnswers,
    ListMyQuestions,
    WithdrawQuestion,
}

/// Every tool, in the order the Python server registers (and lists) them.
pub const ALL: [Name; 6] = [
    Name::AskOperator,
    Name::AlertOperator,
    Name::WithdrawAlert,
    Name::CheckAnswers,
    Name::ListMyQuestions,
    Name::WithdrawQuestion,
];

impl Name {
    /// The tool's name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Name::AskOperator => "ask_operator",
            Name::AlertOperator => "alert_operator",
            Name::WithdrawAlert => "withdraw_alert",
            Name::CheckAnswers => "check_answers",
            Name::ListMyQuestions => "list_my_questions",
            Name::WithdrawQuestion => "withdraw_question",
        }
    }

    /// The tool called `name`, if there is one.
    pub fn parse(name: &str) -> Option<Name> {
        ALL.into_iter().find(|t| t.as_str() == name)
    }

    /// What the tool does, as agents read it.
    pub fn description(self) -> &'static str {
        match self {
            Name::AskOperator => text::ASK,
            Name::AlertOperator => text::ALERT,
            Name::WithdrawAlert => text::WITHDRAW_ALERT,
            Name::CheckAnswers => text::CHECK_ANSWERS,
            Name::ListMyQuestions => text::LIST_MY_QUESTIONS,
            Name::WithdrawQuestion => text::WITHDRAW_QUESTION,
        }
    }

    /// Its arguments' schema.
    pub fn input_schema(self) -> Value {
        match self {
            Name::AskOperator => schema::ask_operator(),
            Name::AlertOperator => schema::alert_operator(),
            Name::WithdrawAlert => schema::by_id("withdraw_alert", false),
            Name::CheckAnswers => schema::check_answers(),
            Name::ListMyQuestions => schema::list_my_questions(),
            Name::WithdrawQuestion => schema::by_id("withdraw_question", true),
        }
    }

    /// The tool as `tools/list` lists it.
    pub fn tool(self) -> Tool {
        let mut t = Tool::new(
            self.as_str(),
            self.description(),
            object(self.input_schema()),
        );
        t.output_schema = Some(object(schema::output(self.as_str())));
        t
    }
}

fn object(v: Value) -> Arc<JsonObject> {
    match v {
        Value::Object(m) => Arc::new(m),
        _ => Arc::default(),
    }
}

/// `tools/list`: every tool.
pub fn list() -> Vec<Tool> {
    ALL.into_iter().map(Name::tool).collect()
}
