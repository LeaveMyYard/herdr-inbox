//! `herdr-inbox hook <event>` run as Claude Code runs it: the built binary,
//! a payload on stdin, a scratch inbox and projects directory, and no
//! `HERDR_*` variable of the caller's (ports of Python tests/test_hook.py and
//! tests/test_hook_delivery.py).

#[path = "../store/common.rs"]
mod common;
mod delivery;
mod run;
mod session_start;
