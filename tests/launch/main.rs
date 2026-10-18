//! The release-path scripts (scripts/launch, scripts/install-bin.sh,
//! scripts/release-lib.sh) run by `sh` in scratch plugin roots, with stand-in
//! binaries, and stand-in curl and cargo (world.rs).

mod fallback;
mod fetch;
mod install;
mod pick;
mod release;
mod world;
