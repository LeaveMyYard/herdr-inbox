//! A scratch plugin root with the real scripts, and stand-in binaries.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The repository (the scripts under test).
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub struct Root {
    pub dir: tempfile::TempDir,
}

impl Root {
    /// A plugin root holding the scripts (and optionally the Python hook stand-in).
    pub fn new() -> Root {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("plugin");
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::create_dir_all(root.join("hooks/fallback")).unwrap();
        for f in [
            "scripts/launch",
            "scripts/install-bin.sh",
            "scripts/release-lib.sh",
            "hooks/fallback/session-start.json",
            "hooks/fallback/subagent-start.json",
        ] {
            fs::copy(repo().join(f), root.join(f)).unwrap();
        }
        fs::create_dir_all(dir.path().join("data/bin")).unwrap();
        fs::create_dir_all(dir.path().join("state/herdr-inbox/bin")).unwrap();
        fake_tools(&dir.path().join("tools"));
        Root { dir }
    }

    /// Where the stand-in curl serves releases from: `<releases>/<tag>/<asset>`.
    pub fn releases(&self) -> PathBuf {
        self.dir.path().join("releases")
    }
    /// Every stand-in curl and cargo call, one line each.
    pub fn tool_log(&self) -> String {
        fs::read_to_string(self.dir.path().join("tools.log")).unwrap_or_default()
    }

    pub fn root(&self) -> PathBuf {
        self.dir.path().join("plugin")
    }
    pub fn data_bin(&self) -> PathBuf {
        self.dir.path().join("data/bin")
    }
    pub fn state_bin(&self) -> PathBuf {
        self.dir.path().join("state/herdr-inbox/bin")
    }

    /// The Python hook stand-in (hooks/run-hook.sh + hooks/run.py).
    pub fn with_python(&self) {
        let hooks = self.root().join("hooks");
        fs::write(hooks.join("run.py"), "").unwrap();
        fs::write(
            hooks.join("run-hook.sh"),
            "echo \"python $2\"; cat; exit 3\n",
        )
        .unwrap();
    }

    /// `sh <root>/scripts/<script> args…` with stdin, in a clean environment.
    pub fn sh(&self, script: &str, args: &[&str], stdin: &str, extra: &[(&str, &Path)]) -> Output {
        let path = format!(
            "{}:{}",
            self.dir.path().join("tools").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new("sh");
        cmd.arg(self.root().join("scripts").join(script))
            .args(args)
            .env_clear()
            .env("PATH", path)
            .env("FAKE_RELEASES", self.releases())
            .env("FAKE_LOG", self.dir.path().join("tools.log"))
            .env("HOME", self.dir.path())
            .env("CLAUDE_PLUGIN_DATA", self.dir.path().join("data"))
            .env("XDG_STATE_HOME", self.dir.path().join("state"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

/// A stand-in binary at `path`: `contract` prints `contract`; no arguments print its
/// version line; `hook <event>` prints "<name> <event>", then its stdin, and exits `code`.
pub fn stand_in(path: &Path, name: &str, version: &str, contract: u32, code: i32) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    // `contract` reads its stdin: a probe that let it would eat the hook's payload
    let script = format!(
        "#!/bin/sh\ncase \"$1\" in\ncontract) cat >/dev/null; echo {contract} ;;\nhook) echo \"{name} $2\"; cat; exit {code} ;;\n'') echo 'herdr-inbox {version}' ;;\nesac\n"
    );
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

/// Stand-ins first on PATH for every script run: `curl` serves only the fixed HTTPS release URL
/// from $FAKE_RELEASES (after $FAKE_CURL_SLEEP seconds, if set) and logs its arguments; `cargo`
/// logs and fails unless $FAKE_CARGO_OK (then `metadata` names $FAKE_TARGET_DIR and `build`
/// puts a stand-in binary there).
fn fake_tools(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    let curl = r#"#!/bin/sh
echo "curl $*" >> "$FAKE_LOG"
out= url=
while [ $# -gt 0 ]; do
	case $1 in
	-o) out=$2; shift ;;
	https://*|http://*|file://*) url=$1 ;;
	esac
	shift
done
[ -n "${FAKE_CURL_SLEEP:-}" ] && sleep "$FAKE_CURL_SLEEP"
case $url in
https://github.com/LeaveMyYard/herdr-inbox/releases/download/*) ;;
*) exit 22 ;;
esac
src="$FAKE_RELEASES/${url#https://github.com/LeaveMyYard/herdr-inbox/releases/download/}"
[ -f "$src" ] || exit 22
cp "$src" "$out"
"#;
    let cargo = r#"#!/bin/sh
echo "cargo $*" >> "$FAKE_LOG"
[ -n "${FAKE_CARGO_OK:-}" ] || exit 101
case $1 in
metadata) printf '{"target_directory":"%s"}\n' "$FAKE_TARGET_DIR" ;;
build)
	mkdir -p "$FAKE_TARGET_DIR/release"
	printf '#!/bin/sh\n[ "$1" = contract ] && echo 1 && exit 0\necho "herdr-inbox 0.0.7"\n' > "$FAKE_TARGET_DIR/release/herdr-inbox"
	chmod +x "$FAKE_TARGET_DIR/release/herdr-inbox"
	;;
esac
"#;
    for (name, body) in [("curl", curl), ("cargo", cargo)] {
        fs::write(dir.join(name), body).unwrap();
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }
}
