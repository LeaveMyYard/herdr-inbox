//! The inbox's marks on herdr, from the command line (Python
//! `cli._command`): `inbox sync-tokens [--remove-prefixes]` reconciles every
//! pane token and tab prefix; `inbox radar-sidebar [--remove]` puts
//! `$inbox_q` into herdr-radar's sidebar rows (or takes it out).

use super::args::parse::Args;
use super::{Io, warner};
use crate::radar;
use crate::store::Store;
use crate::surface::markers::Markers;

/// `inbox sync-tokens`: silent, 0.
pub fn sync_tokens(store: &Store, args: &Args, io: &mut Io<'_>) -> rusqlite::Result<i32> {
    let synced = {
        let mut warn = warner(&mut *io.err);
        let mut markers = Markers {
            store,
            db_path: io.db_path,
            herdr: &mut *io.herdr,
            config: &mut *io.config,
            lock_wait: io.lock_wait,
            warn: &mut warn,
        };
        markers
            .sync(args.has("--remove-prefixes"))
            .map_err(|e| e.to_string())
    };
    if let Err(err) = synced {
        let _ = writeln!(io.err, "inbox: {err}"); // Python's traceback, as one line
        return Ok(1);
    }
    Ok(0)
}

/// `inbox radar-sidebar`: what happened, in Python's words; 0 when the rows
/// are as asked.
pub fn radar_sidebar(args: &Args, io: &mut Io<'_>) -> i32 {
    let remove = args.has("--remove");
    let mut warn = warner(&mut *io.err);
    let status = radar::ensure(io.herdr_config, remove, &mut *io.herdr, &mut warn);
    let _ = writeln!(io.out, "{}", status.message(io.herdr_config, remove));
    status.code()
}
