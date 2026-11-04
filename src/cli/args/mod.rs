//! The CLI's arguments, parsed as Python's `argparse` parses `inbox`'s
//! (`cli.main`): the same accepted spellings, the same help, and the same
//! usage errors on stderr with exit code 2.
//!
//! - [`parse`]: one subcommand's options and positionals, in argparse's
//!   order (an error is reported where argparse meets it).
//! - question ids are [`crate::deeplink::qref`]'s (`Q12`, `q12`, `12`).
//! - [`pyrepr`]: Python's `repr()` of a string, as the errors quote values.

pub mod parse;
pub mod pyrepr;

pub use crate::deeplink::qref::{question_id, question_ref};

/// `inbox`'s own usage, as argparse renders it without a terminal (80
/// columns): every usage error of a subcommand's extra arguments prints it.
pub const TOP_USAGE: &str = "usage: inbox [-h]\n             {mcp,tui,open,list,answer,dismiss,redeliver,sync-tokens,radar-sidebar,clean-venvs,deliver,verify,worker,export,apply,remote,hook} ...\n";
