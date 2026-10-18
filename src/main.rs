//! The `herdr-inbox` binary (plan §2.2): one binary, subcommands keeping
//! today's CLI names. So far: `hook <event>` (the Claude Code hooks).
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
        _ => {
            println!("herdr-inbox {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
    }
}
