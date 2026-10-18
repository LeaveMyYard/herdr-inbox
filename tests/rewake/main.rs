//! The rewake watcher against a scripted herdr (FakeHerdr) and a scratch
//! inbox, on a fake clock (ports of Python tests/test_rewake.py).

#[path = "../store/common.rs"]
mod common;
mod endable;
mod locks;
mod panes;
mod scenarios;
mod world;
