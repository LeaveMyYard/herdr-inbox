//! The CLI (`herdr_inbox::cli`) against real databases: the golden Python's
//! `cli.main` left (tests/fixtures/cli.json), and the live wiring over a
//! scripted herdr socket.

#[path = "../store/common.rs"]
mod common;
mod deeplink;
mod fakes;
mod golden;
#[path = "../delivery/herdr_stub.rs"]
mod herdr_stub;
mod live;
mod radar;
#[path = "../store/golden.rs"]
#[allow(dead_code)]
mod store_golden;
mod tabs;
