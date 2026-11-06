//! Where the core's socket is: `$HC_SOCKET`, else
//! `<runtime>/herdr-core/<session>.sock`, the runtime directory being
//! `$XDG_RUNTIME_DIR`, else `/run/user/<uid>`, and the session `$HC_SESSION`,
//! else `default`.
//!
//! This is part of hc's rule. `HC_RUNTIME` and a socket in hc's config are not
//! read. A core placed by those, or a place this cannot name (`/run/user` is
//! Linux's), reads as no core: no `hc`, so `unavailable`, never a wrong one.

use std::path::{Path, PathBuf};

use herdr_agent_kit::plugin_env::PluginEnv;

/// This process's core socket.
pub fn from_env(env: &PluginEnv) -> Option<PathBuf> {
    if let Some(socket) = env.path_var("HC_SOCKET") {
        return socket.is_absolute().then(|| socket.to_owned());
    }
    let uid = rustix::process::getuid().as_raw();
    let runtime = env.path_var("XDG_RUNTIME_DIR");
    path(
        runtime,
        env.var("HC_SESSION"),
        uid,
        cfg!(target_os = "linux"),
    )
}

fn path(runtime: Option<&Path>, session: Option<&str>, uid: u32, linux: bool) -> Option<PathBuf> {
    let runtime = match runtime {
        Some(dir) if dir.is_absolute() => dir.to_owned(),
        Some(_) => return None,
        None if linux => PathBuf::from(format!("/run/user/{uid}")),
        None => return None,
    };
    let session = session.unwrap_or("default");
    let plain = |b: u8| b.is_ascii_alphanumeric() || b"._-".contains(&b);
    (!session.is_empty() && !session.starts_with('.') && session.bytes().all(plain))
        .then(|| runtime.join("herdr-core").join(format!("{session}.sock")))
}

#[cfg(test)]
mod tests;
