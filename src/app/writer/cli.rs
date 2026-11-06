//! `herdr-inbox app-feed`: `--help`, `--ensure [--app-home ABSOLUTE_HOME]`,
//! and `--writer`, which `--ensure` runs and nothing else should.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use herdr_agent_kit::internal::plugin_paths::inbox;
use herdr_agent_kit::plugin_env::PluginEnv;

use super::ensure::ensure;
use super::files::Paths;
use super::run::serve;
use crate::worker::process::own_exe;

/// The first line of `--help`, which a mod's probe reads.
pub const HELP: &str = "herdr-inbox app-feed v1\n\
usage: herdr-inbox app-feed --ensure [--app-home ABSOLUTE_HOME]\n";

#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Help,
    Ensure(Option<PathBuf>),
    Writer(Option<PathBuf>),
}

fn parse(args: &[String]) -> Option<Parsed> {
    if matches!(args, [only] if only == "--help") {
        return Some(Parsed::Help);
    }
    let (mut ensure, mut writer, mut home) = (false, false, None);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--ensure" if !ensure => ensure = true,
            "--writer" if !writer => writer = true,
            "--app-home" if home.is_none() => {
                let path = PathBuf::from(rest.next()?);
                home = Some(path).filter(|p| p.is_absolute());
                home.as_ref()?;
            }
            _ => return None,
        }
    }
    match (ensure, writer) {
        (true, false) => Some(Parsed::Ensure(home)),
        (false, true) => Some(Parsed::Writer(home)),
        _ => None,
    }
}

/// `<appHome>`: `$HERDR_APP_HOME`, else `$HOME/.config/herdr-app`, as the
/// app's own `appHome()`. A relative one names no place.
fn app_home(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |name| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = set("HERDR_APP_HOME").or_else(|| Some(set("HOME")?.join(".config/herdr-app")))?;
    Some(home).filter(|p| p.is_absolute())
}

/// The subcommand; its exit code.
pub fn main(args: &[String]) -> i32 {
    let Some(parsed) = parse(args) else {
        let _ = write!(std::io::stderr(), "{HELP}");
        return 2;
    };
    match parsed {
        Parsed::Help => {
            print!("{HELP}");
            0
        }
        Parsed::Ensure(home) => with_home(home, |paths, exe, _| match ensure(paths, exe) {
            Ok(_) => 0,
            Err(why) => refuse(&why),
        }),
        Parsed::Writer(home) => with_home(home, serve),
    }
}

fn with_home(home: Option<PathBuf>, run: impl FnOnce(&Paths, &Path, &Path) -> i32) -> i32 {
    let Some(home) = home.or_else(|| app_home(|name| std::env::var_os(name))) else {
        return refuse("no app home: neither --app-home, HERDR_APP_HOME nor HOME");
    };
    let db = inbox::db_path(&PluginEnv::from_env());
    run(&Paths::new(&home), &own_exe(), &db)
}

fn refuse(why: &str) -> i32 {
    let _ = writeln!(std::io::stderr(), "herdr-inbox app-feed: {why}");
    1
}

#[cfg(test)]
mod tests;
