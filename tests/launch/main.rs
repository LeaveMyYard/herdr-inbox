//! The release-path scripts (scripts/launch, scripts/install-bin.sh,
//! scripts/release-lib.sh) run by `sh` in scratch plugin roots, with stand-in
//! binaries and a `file://` release.

mod fallback;
mod install;
mod pick;
mod world;
