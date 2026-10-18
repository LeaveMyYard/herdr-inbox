# herdr-inbox: the herdr build step (herdr-plugin.toml [[build]]; plan §4 item 3).
#
#   sh scripts/install-bin.sh
#
# Run by `herdr plugin install` / `plugin update` in the new install directory. Puts the binary
# at <plugin dir>/bin/herdr-inbox:
# - the release named in release.toml, for this machine's target, checked against its sha256
#   (release.toml was committed after the release was published, so an asset replaced later
#   fails the install);
# - with no matching asset (no release.toml yet, an unsupported target, a failed download),
#   `cargo build --release --locked` when cargo exists; otherwise a clear error.
# A checksum mismatch is an error, never a reason to build instead.
#
# Then it records a hard copy for the Claude Code side, outside any install directory (herdr
# deletes old ones after an update): ${XDG_STATE_HOME:-~/.local/state}/herdr-inbox/bin/
# herdr-inbox-<version>, where scripts/launch looks.
set -u
here=$(cd "$(dirname "$0")/.." && pwd) || exit 1
cd "$here" || exit 1
# shellcheck source=scripts/release-lib.sh
. "$here/scripts/release-lib.sh"

bin="$here/bin/herdr-inbox"
inbox_fetch "$here/release.toml" "$bin"
case $? in
0) ;;
3)
	echo "herdr-inbox: the downloaded release does not match release.toml's sha256; refusing it" >&2
	exit 1
	;;
*)
	if ! command -v cargo >/dev/null 2>&1; then
		echo "herdr-inbox: no release binary for this machine ($(uname -s) $(uname -m)) and no cargo to build one; install Rust (https://rustup.rs) and run 'herdr plugin update'" >&2
		exit 1
	fi
	cargo build --release --locked || exit 1
	mkdir -p "$here/bin" || exit 1
	built="${CARGO_TARGET_DIR:-target}/release/herdr-inbox"
	cp "$built" "$bin.tmp.$$" && mv -f "$bin.tmp.$$" "$bin" || exit 1
	;;
esac

version=$(inbox_version_of "$bin")
if [ -z "$version" ]; then
	echo "herdr-inbox: $bin does not run" >&2
	exit 1
fi
state="${XDG_STATE_HOME:-$HOME/.local/state}/herdr-inbox/bin"
if mkdir -p "$state" && cp "$bin" "$state/.herdr-inbox-$version.$$" && mv -f "$state/.herdr-inbox-$version.$$" "$state/herdr-inbox-$version"; then
	:
else
	rm -f "$state/.herdr-inbox-$version.$$"
	echo "herdr-inbox: could not record the binary for Claude Code in $state (its hooks fall back to Python)" >&2
fi
exit 0
