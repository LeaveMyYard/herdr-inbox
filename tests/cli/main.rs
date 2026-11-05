//! The CLI (`herdr_inbox::cli`) against real databases: the golden Python's
//! `cli.main` left (tests/fixtures/cli.json), and the live wiring over a
//! scripted herdr socket.

#[path = "../store/common.rs"]
mod common;
mod deeplink;
mod fakes;
mod golden;
#[path = "../support/herdr_cli.rs"]
mod herdr_cli;
#[path = "../delivery/herdr_stub.rs"]
mod herdr_stub;
mod live;
mod radar;
mod remote;
mod remote_e2e;
#[path = "../store/golden.rs"]
#[allow(dead_code)]
mod store_golden;
mod tabs;
