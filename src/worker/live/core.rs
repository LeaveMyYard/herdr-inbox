//! What the core worker's life depends on: herdr-core, not a herdr server.
//!
//! One core worker per machine, keyed on [`CORE_SERVER`]: `ensure_worker` hands
//! that word over as the worker's server, through `HERDR_SOCKET_PATH` as for any
//! other, and that word is how the worker knows what it is. It is no socket, so
//! nothing here asks herdr.
//!
//! [`CORE_SERVER`]: crate::store::hosts::CORE_SERVER

use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crate::worker::spawn::runs_rust;

/// The core this worker lives with, and the install it came from.
#[derive(Clone, Debug)]
pub struct Core {
    /// The core's socket, if its place is known.
    pub socket: Option<PathBuf>,
    /// This worker's checkout.
    pub root: Option<PathBuf>,
}

impl Core {
    /// Is this worker done? The core is proved gone: its place is unknown, its
    /// socket is missing or refuses (any other failure is no proof). Or its install
    /// went back to Python, where core rows are held and nothing may type them.
    pub fn gone(&self) -> bool {
        self.root.as_deref().is_some_and(|r| !runs_rust(r))
            || self.socket.as_deref().is_none_or(absent)
    }
}

/// Nobody listens at `socket`.
pub fn absent(socket: &Path) -> bool {
    match UnixStream::connect(socket) {
        Ok(_) => false,
        Err(e) => matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused),
    }
}
