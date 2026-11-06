# herdr-inbox: the herdr build step (herdr-plugin.toml [[build]]; plan §4 item 3).
#
#   sh scripts/install-bin.sh           what herdr runs
#   sh scripts/install-bin.sh --build   by hand, in a checkout: build this commit and record it
#
# Run by `herdr plugin install` / `plugin update` in the new install directory. Puts the binary
# at <plugin dir>/bin/herdr-inbox and records a hard copy for the Claude Code side, outside any
# install directory (herdr deletes old ones after an update):
# ${XDG_STATE_HOME:-~/.local/state}/herdr-inbox/bin/herdr-inbox-<version>, where scripts/launch
# looks. Older recorded copies beyond the newest three of each contract are removed then.
#
# - Before the first release (no release.toml) it does nothing: Python stays in use.
# - Otherwise the release named in release.toml, for this machine's target, checked against its
#   sha256 (release.toml was committed after the release was published, so an asset replaced
#   later fails the install). A checksum mismatch, or no sha256 tool to check with, fails the
#   install: nothing unverified is ever installed, and nothing is built around it.
# - A failed download installs nothing (Python stays in use; the next update retries).
# - Only with no asset for this machine at all (a target the release does not build) does it
#   build this commit with `cargo build --release --locked`. No cargo, or a build that fails, is
#   never an error: Python stays in use.
# - --build skips the release and builds this commit, so a checkout can go live with no release
#   out, and the Claude Code side finds the binary too. There the operator asked for a binary,
#   so a failure exits 1. Python still stays in use meanwhile.
set -u
here=$(cd "$(dirname "$0")/.." && pwd) || exit 0
cd "$here" || exit 0
# shellcheck source=scripts/release-lib.sh
. "$here/scripts/release-lib.sh"

bin="$here/bin/herdr-inbox"

# Builds this commit into $bin; $1 says why, for the message when that fails. 1: it failed.
build_here() {
	if ! command -v cargo >/dev/null 2>&1; then
		echo "herdr-inbox: $1 and no cargo to build one: Python stays in use" >&2
		return 1
	fi
	# the build's own target directory (a global build.target-dir or CARGO_TARGET_DIR included)
	target_dir=$(cargo metadata --format-version 1 --no-deps 2>/dev/null | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
	if ! cargo build --release --locked || [ ! -x "${target_dir:-target}/release/herdr-inbox" ]; then
		echo "herdr-inbox: $1 and the build failed: Python stays in use" >&2
		return 1
	fi
	if ! { mkdir -p "$here/bin" && cp "${target_dir:-target}/release/herdr-inbox" "$bin.tmp.$$" && mv -f "$bin.tmp.$$" "$bin"; }; then
		rm -f "$bin.tmp.$$"
		return 1
	fi
}

if [ "${1:-}" = --build ]; then
	build_here "asked to build this commit" || exit 1
else
	if [ ! -f "$here/release.toml" ]; then
		echo "herdr-inbox: no Rust release yet: Python stays in use" >&2
		exit 0
	fi
	inbox_fetch "$here/release.toml" "$bin"
	case $? in
	0) ;;
	3)
		echo "herdr-inbox: the downloaded release does not match release.toml's sha256; refusing it" >&2
		exit 1
		;;
	4)
		echo "herdr-inbox: no sha256sum or shasum to verify the release with; refusing to install it unverified" >&2
		exit 1
		;;
	2)
		echo "herdr-inbox: the release download failed: Python stays in use (run 'herdr plugin update' to retry)" >&2
		exit 0
		;;
	*)
		build_here "no release binary for this machine ($(uname -s) $(uname -m))" || exit 0
		;;
	esac
fi

version=$(inbox_version_of "$bin")
if [ -z "$version" ]; then
	echo "herdr-inbox: $bin does not run: Python stays in use" >&2
	exit 0
fi
if [ -n "${XDG_STATE_HOME:-}" ]; then
	state="$XDG_STATE_HOME/herdr-inbox/bin"
elif [ -n "${HOME:-}" ]; then
	state="$HOME/.local/state/herdr-inbox/bin"
else
	echo "herdr-inbox: no XDG_STATE_HOME or HOME: the binary is not recorded for Claude Code (it falls back to Python)" >&2
	exit 0
fi
if mkdir -p "$state" && cp "$bin" "$state/.herdr-inbox-$version.$$" && mv -f "$state/.herdr-inbox-$version.$$" "$state/herdr-inbox-$version"; then
	# one copy per release, forever, otherwise (#72): the newest three of each contract stay
	inbox_prune "$state" 3 "$state/herdr-inbox-$version"
else
	rm -f "$state/.herdr-inbox-$version.$$"
	echo "herdr-inbox: could not record the binary for Claude Code in $state (it falls back to Python)" >&2
fi
exit 0
