//! `herdr-inbox hook <event>`: the Claude Code hooks (Python `hook.run`).
//!
//! A hook must be fast and must never fail the session: anything unexpected
//! means exit 0 with no output. Only the pane's own agent gets the inbox
//! (`HERDR_PANE_ID` set, not a Claude nested in another).

pub mod deliver;
pub mod output;
pub mod payload;
pub mod rules;
pub mod session_start;

use std::io::{Read, Write};

use herdr_agent_kit::agent_procs::claude::nested_claude_here;
use herdr_agent_kit::internal::plugin_paths::inbox;
use herdr_agent_kit::plugin_env::PluginEnv;

use crate::claude::Transcripts;
use crate::store::db_path;
use output::hook_output;
use payload::Payload;
use rules::SUBAGENT_RULES;

/// What a hook run ends with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Printed to stdout (one line), if anything.
    pub stdout: Option<String>,
    /// The exit code: 0, or the rewake watcher's wake code.
    pub code: i32,
}

impl Outcome {
    fn nothing() -> Outcome {
        Outcome {
            stdout: None,
            code: 0,
        }
    }
}

/// Runs hook `event` with `stdin` as the payload in this process's
/// environment, and prints its output. The exit code to use is returned.
pub fn main(event: &str) -> i32 {
    let env = PluginEnv::from_env();
    if env.pane_id().is_none() || nested_claude_here() {
        return 0; // not the pane's own agent: nobody watches the inbox for it
    }
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return 0;
    }
    let outcome = run(event, &raw, &env, &Transcripts::from_env());
    if let Some(text) = &outcome.stdout {
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{text}").and_then(|()| out.flush()).is_err() && outcome.code != 0 {
            return 0; // never a wake without its text (Python returns 0)
        }
    }
    if event == "session-start"
        && let Some(payload) = Payload::parse(&raw)
    {
        session_start::remember_transcript(&db_path(&env), &payload);
    }
    outcome.code
}

/// One hook run on `raw` (the payload text), without printing: what to print
/// and the exit code.
pub fn run(event: &str, raw: &str, env: &PluginEnv, files: &Transcripts) -> Outcome {
    let Some(payload) = Payload::parse(raw) else {
        return Outcome::nothing();
    };
    let db = db_path(env);
    let (name, context) = match event {
        "session-start" => {
            session_start::mark_resume(&inbox::state_dir(env), &payload, env.pane_id());
            ("SessionStart", session_start::context(&db, &payload))
        }
        "subagent-start" => ("SubagentStart", SUBAGENT_RULES.to_owned()),
        "post-tool-batch" | "stop" => {
            let name = if event == "stop" {
                "Stop"
            } else {
                "PostToolBatch"
            };
            match deliver::deliver(&db, files, &payload, true) {
                Some(text) => (name, text),
                None => return Outcome::nothing(),
            }
        }
        "rewake-watch" => {
            // asyncRewake: the text itself on stdout, and the wake exit
            let clock = crate::rewake::SystemClock::new();
            let watcher = crate::rewake::Watcher::from_env(env, &clock);
            return match crate::rewake::run(&watcher, &payload) {
                Some(text) => Outcome {
                    stdout: Some(text),
                    code: crate::rewake::WAKE_EXIT,
                },
                None => Outcome::nothing(),
            };
        }
        _ => return Outcome::nothing(),
    };
    Outcome {
        stdout: Some(hook_output(name, &context)),
        code: 0,
    }
}
