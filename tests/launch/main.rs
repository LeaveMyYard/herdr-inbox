//! The release-path scripts (scripts/launch, scripts/install-bin.sh,
//! scripts/release-lib.sh) run by `sh` in scratch plugin roots, with stand-in
//! binaries, and stand-in curl and cargo (world.rs).

mod fallback;
mod fetch;
#[path = "../support/herdr_cli.rs"]
mod herdr_cli;
mod install;
mod manifest;
mod pick;
mod prune;
mod release;
mod world;
