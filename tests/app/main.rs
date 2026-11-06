//! The herdr-app contract against the code: the feed of each fixture's world
//! is that fixture, and the writer does what the documents say.
//!
//! No test here can reach a real herdr, core or app: a process started here has a cleared
//! environment, a `PATH` of the rig's stand-ins only, and private homes ([`rig`]).

mod feed;
mod rig;
mod runtime;
mod world;
