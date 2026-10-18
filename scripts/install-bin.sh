# herdr-inbox: the herdr build step (herdr-plugin.toml [[build]]; plan §4 item 3).
#
#   sh scripts/install-bin.sh
#
# Run by `herdr plugin install` / `plugin update` in the new install directory. Puts the binary
# at <plugin dir>/bin/herdr-inbox and records a hard copy for the Claude Code side, outside any
# install directory (herdr deletes old ones after an update):
# ${XDG_STATE_HOME:-~/.local/state}/herdr-inbox/bin/herdr-inbox-<version>, where scripts/launch
# looks.
#
# - Before the first release (no release.toml) it does nothing: the Python hooks stay in use.
# - Otherwise the release named in release.toml, for this machine's target, checked against its
#   sha256 (release.toml was committed after the release was published, so an asset replaced
#   later fails the install). A checksum mismatch, or no sha256 tool to check with, fails the
#   install: nothing unverified is ever installed, and nothing is built around it.
# - A failed download installs nothing (the Python hooks stay in use; the next update retries).
# - Only with no asset for this machine at all (a target the release does not build) does it
#   build this commit with `cargo build --release --locked`. No cargo, or a build that fails, is
#   never an error: the Python hooks stay in use.
set -u
here=$(cd "$(dirname "$0")/.." && pwd) || exit 0
cd "$here" || exit 0
# shellcheck source=scripts/release-lib.sh
. "$here/scripts/release-lib.sh"

if [ ! -f "$here/release.toml" ]; then
	echo "herdr-inbox: no Rust release yet: the Python hooks stay in use" >&2
	exit 0
fi

bin="$here/bin/herdr-inbox"
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
	echo "herdr-inbox: the release download failed: the Python hooks stay in use (run 'herdr plugin update' to retry)" >&2
	exit 0
	;;
*)
	why="no release binary for this machine ($(uname -s) $(uname -m))"
	if ! command -v cargo >/dev/null 2>&1; then
		echo "herdr-inbox: $why and no cargo to build one: the Python hooks stay in use" >&2
		exit 0
	fi
	# the build's own target directory (a global build.target-dir or CARGO_TARGET_DIR included)
	target_dir=$(cargo metadata --format-version 1 --no-deps 2>/dev/null | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
	if ! cargo build --release --locked || [ ! -x "${target_dir:-target}/release/herdr-inbox" ]; then
		echo "herdr-inbox: $why and the build failed: the Python hooks stay in use" >&2
		exit 0
	fi
	if ! { mkdir -p "$here/bin" && cp "${target_dir:-target}/release/herdr-inbox" "$bin.tmp.$$" && mv -f "$bin.tmp.$$" "$bin"; }; then
		rm -f "$bin.tmp.$$"
		exit 0
	fi
	;;
esac

version=$(inbox_version_of "$bin")
if [ -z "$version" ]; then
	echo "herdr-inbox: $bin does not run: the Python hooks stay in use" >&2
	exit 0
fi
if [ -n "${XDG_STATE_HOME:-}" ]; then
	state="$XDG_STATE_HOME/herdr-inbox/bin"
elif [ -n "${HOME:-}" ]; then
	state="$HOME/.local/state/herdr-inbox/bin"
else
	echo "herdr-inbox: no XDG_STATE_HOME or HOME: the binary is not recorded for Claude Code (its hooks fall back to Python)" >&2
	exit 0
fi
if mkdir -p "$state" && cp "$bin" "$state/.herdr-inbox-$version.$$" && mv -f "$state/.herdr-inbox-$version.$$" "$state/herdr-inbox-$version"; then
	:
else
	rm -f "$state/.herdr-inbox-$version.$$"
	echo "herdr-inbox: could not record the binary for Claude Code in $state (its hooks fall back to Python)" >&2
fi
exit 0
