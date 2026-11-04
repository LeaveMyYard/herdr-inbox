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

# Removes inbox_fetch's temporary directory and its traps; returns $1.
_inbox_drop() {
	rm -rf "$_tmp"
	trap - EXIT HUP INT TERM
	return "$1"
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
	# an interrupted fetch leaves no temporary directory behind
	trap 'rm -rf "$_tmp"' EXIT
	trap 'rm -rf "$_tmp"; exit 1' HUP INT TERM
	if ! curl -fsSL --proto '=https' --proto-redir '=https' --connect-timeout 10 --max-time 300 --retry 2 \
		-o "$_tmp/t.tar.gz" "$INBOX_RELEASES/$_tag/herdr-inbox-$_tag-$_target.tar.gz"; then
		_inbox_drop 2
		return
	fi
	_got=$(inbox_sha256 "$_tmp/t.tar.gz")
	if [ -z "$_got" ]; then
		_inbox_drop 4
		return
	fi
	if [ "$_got" != "$_want" ]; then
		_inbox_drop 3
		return
	fi
	if ! tar -C "$_tmp" -xzf "$_tmp/t.tar.gz" herdr-inbox || [ ! -f "$_tmp/herdr-inbox" ]; then
		_inbox_drop 2
		return
	fi
	# a rename: whoever runs the old file keeps its inode; the new one appears whole
	chmod 0755 "$_tmp/herdr-inbox" && mv -f "$_tmp/herdr-inbox" "$_dest"
	_rc=$?
	_inbox_drop 0
	[ "$_rc" -eq 0 ] || return 2
}

# inbox_versions <dir>...: the recorded binaries (herdr-inbox-X.Y.Z) in the directories, as
# "major<TAB>minor<TAB>patch<TAB>path", newest first (anything else in them is skipped).
inbox_versions() {
	_t=$(printf '\t')
	for _d in "$@"; do
		for _f in "$_d"/herdr-inbox-*; do
			[ -f "$_f" ] || continue
			_v=${_f##*/herdr-inbox-}
			case $_v in
			*[!0-9.]* | .* | *.) continue ;;
			esac
			printf '%s\t%s\n' "$_v" "$_f" | awk -F '\t' '{ n = split($1, v, "."); if (n == 3) printf "%d\t%d\t%d\t%s\n", v[1], v[2], v[3], $2 }'
		done
	done | sort -t "$_t" -k1,1nr -k2,2nr -k3,3nr
}

# inbox_prune <dir> <keep> [<path>]: removes the recorded binaries of <dir> beyond the newest
# <keep> of each contract (a Claude plugin of an older contract still finds its own), never
# <path> (the one just recorded). Removing a file a process runs is safe: it keeps its inode;
# and a launcher only ever picks the newest of its contract, which stays (#72).
inbox_prune() {
	[ -d "$1" ] || return 0
	_seen=$(mktemp "$1/.prune.XXXXXX") || return 0
	inbox_versions "$1" | cut -f 4- | while IFS= read -r _f; do
		_c=$("$_f" contract 2>/dev/null </dev/null | head -n 1)
		_n=$(grep -cxF "c$_c" "$_seen")
		printf 'c%s\n' "$_c" >>"$_seen"
		if [ "$_n" -ge "$2" ] && [ "$_f" != "${3:-}" ]; then
			rm -f "$_f"
		fi
	done
	rm -f "$_seen"
}
