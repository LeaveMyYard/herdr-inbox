//! The herdr-app contract against the code: the feed of each fixture's world
//! is that fixture, and the verbs and the writer do what the documents say.
//!
//! No test here can reach a real herdr, core or app: a process started here has a cleared
//! environment, a `PATH` of the rig's stand-ins only, and private homes ([`rig`]).

mod commands;
mod feed;
mod live;
mod rig;
mod runtime;
mod verbs;
mod world;
