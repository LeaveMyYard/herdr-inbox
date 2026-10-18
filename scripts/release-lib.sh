# herdr-inbox: shared by scripts/install-bin.sh and scripts/launch (sourced, POSIX sh).
# The release flow is plan §4 (herdr-panorama docs/plans/rust-rewrite.md): release.toml names
# the release to fetch and the sha256 of each target's tarball.

# Fixed, HTTPS only: never taken from the environment.
INBOX_RELEASES=https://github.com/LeaveMyYard/herdr-inbox/releases/download

# The release target of this machine, or nothing (an unsupported one).
inbox_target() {
	case "$(uname -s) $(uname -m)" in
	"Linux x86_64" | "Linux amd64") echo x86_64-unknown-linux-musl ;;
	"Linux aarch64" | "Linux arm64") echo aarch64-unknown-linux-musl ;;
	"Darwin x86_64") echo x86_64-apple-darwin ;;
	"Darwin arm64" | "Darwin aarch64") echo aarch64-apple-darwin ;;
	esac
}

# The value of `key = "value"` in release.toml ($1), or nothing.
inbox_release_value() {
	sed -n "s/^$2 = \"\\([^\"]*\\)\"\$/\\1/p" "$1" 2>/dev/null | head -n 1
}

# The sha256 of a file, lowercase hex, or nothing.
inbox_sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d ' ' -f 1
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | cut -d ' ' -f 1
	fi
}

# The version a binary reports (`herdr-inbox X.Y.Z`), or nothing.
inbox_version_of() {
	"$1" 2>/dev/null </dev/null | sed -n 's/^herdr-inbox \([0-9][0-9.]*\)$/\1/p' | head -n 1
}

# inbox_fetch <release.toml> <dest binary>: download this machine's tarball of the release,
# check it against release.toml's sha256, and move the binary into <dest> atomically.
# Returns 1 when there is nothing to fetch (no release.toml, no tag, an unsupported target),
# 2 when the download fails, 3 when the checksum does not match, 4 when no sha256 tool exists
# (nothing can be verified, so nothing is installed); a failed fetch leaves no file behind.
inbox_fetch() {
	_toml=$1
	_dest=$2
	[ -r "$_toml" ] || return 1
	_tag=$(inbox_release_value "$_toml" tag)
	_target=$(inbox_target)
	[ -n "$_tag" ] && [ -n "$_target" ] || return 1
	_want=$(inbox_release_value "$_toml" "$_target")
	[ -n "$_want" ] || return 1
	_dir=$(dirname "$_dest")
	mkdir -p "$_dir" || return 2
	_tmp=$(mktemp -d "$_dir/.fetch.XXXXXX") || return 2
	if ! curl -fsSL --proto '=https' --proto-redir '=https' --connect-timeout 10 --max-time 300 --retry 2 \
		-o "$_tmp/t.tar.gz" "$INBOX_RELEASES/$_tag/herdr-inbox-$_tag-$_target.tar.gz"; then
		rm -rf "$_tmp"
		return 2
	fi
	_got=$(inbox_sha256 "$_tmp/t.tar.gz")
	if [ -z "$_got" ]; then
		rm -rf "$_tmp"
		return 4
	fi
	if [ "$_got" != "$_want" ]; then
		rm -rf "$_tmp"
		return 3
	fi
	if ! tar -C "$_tmp" -xzf "$_tmp/t.tar.gz" herdr-inbox || [ ! -f "$_tmp/herdr-inbox" ]; then
		rm -rf "$_tmp"
		return 2
	fi
	chmod 0755 "$_tmp/herdr-inbox" && mv -f "$_tmp/herdr-inbox" "$_dest"
	_rc=$?
	rm -rf "$_tmp"
	[ "$_rc" -eq 0 ] || return 2
}
