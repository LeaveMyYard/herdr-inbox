//! The `herdr-inbox` binary (plan §2.2): one binary, subcommands keeping
//! today's CLI names. So far: `hook <event>` (the Claude Code hooks), `mcp`
//! (the MCP stdio server; nothing launches it before P5-I), `worker
//! [--ensure]` (the delivery worker, [`herdr_inbox::worker::process`]; the
//! Python bridge hands over to it only from an install that has this
//! binary built, which no release has before P5-I), `contract` (the
//! launcher contract, `scripts/launch`) and the CLI's `list`, `answer` and
//! `dismiss` ([`herdr_inbox::cli`]).
//!
//! A hook never fails the session: a panic, a bad argument or any error
//! exits 0 (the rewake watcher's wake code is the only other exit).

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, event] if cmd == "hook" => {
            std::panic::set_hook(Box::new(|_| {})); // stderr is discarded; stay silent
            let code = std::panic::catch_unwind(|| herdr_inbox::hook::main(event)).unwrap_or(0);
            ExitCode::from(u8::try_from(code).unwrap_or(0))
        }
        [cmd, ..] if cmd == "hook" => ExitCode::SUCCESS,
        [cmd, rest @ ..] if cmd == "worker" => {
            ExitCode::from(u8::try_from(herdr_inbox::worker::process::main(rest)).unwrap_or(1))
        }
        [cmd] if cmd == "mcp" => ExitCode::from(u8::try_from(herdr_inbox::mcp::run()).unwrap_or(1)),
        [cmd, rest @ ..] if herdr_inbox::cli::handles(cmd) => {
            let code = herdr_inbox::cli::live::main(cmd, rest);
            ExitCode::from(u8::try_from(code).unwrap_or(1))
        }
        [cmd] if cmd == "contract" => {
            println!("{}", herdr_inbox::contract::CONTRACT);
            ExitCode::SUCCESS
        }
        _ => {
            println!("herdr-inbox {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
    }
}
