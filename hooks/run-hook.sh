# herdr-inbox: run hooks/run.py with a cached interpreter instead of the `python3` on PATH.
#
#   sh run-hook.sh <run.py> <event>
#
# `python3` is often a version-manager shim (pyenv's is a bash script costing 60-75 ms per
# call, on every PostToolBatch and Stop). The interpreter it resolves to is cached in
# Claude Code's per-plugin data dir ($CLAUDE_PLUGIN_DATA/python-interpreter, kept across
# plugin updates) and executed directly afterwards. The hot path is one file read.
#
# Safe by construction:
# - A cache that is unreadable, not an absolute path, not named python*, or no longer an
#   executable regular file is ignored and re-resolved, so an interpreter that disappears
#   (pyenv uninstall, brew upgrade) invalidates itself.
# - Only an interpreter run.py can use directly (3.11+) is cached. pyenv picks the version
#   per directory (.python-version), so a project pinned to an older Python must not pin
#   every session onto run.py's uv fallback; such a call just runs `python3` as before.
# - The cache is written to a per-process temp file and renamed into place: concurrent
#   hooks only ever read a whole file, and they all write the same answer.
# - No CLAUDE_PLUGIN_DATA, an unwritable dir, or a broken `python3` all end in plain
#   `python3 -I run.py <event>`, exactly the command before this script existed.
# - The resolution reads no stdin (the hook payload stays for run.py) and prints nothing.
#   hooks.json discards stderr and decides the exit code around this script; the only exit
#   status that passes through is run.py's own (3 = wake, rewake-watch only).

hook=$1
shift

usable() {
	case $1 in
	/*/python* | /*/Python*) [ -f "$1" ] && [ -x "$1" ] ;;
	*) return 1 ;;
	esac
}

if [ -n "${CLAUDE_PLUGIN_DATA:-}" ]; then
	cache=$CLAUDE_PLUGIN_DATA/python-interpreter
	py=
	[ -r "$cache" ] && IFS= read -r py <"$cache"
	if usable "$py"; then
		exec "$py" -I "$hook" "$@"
	fi
	py=$(python3 -I -c 'import sys; print(sys.executable if sys.version_info >= (3, 11) else "")' 2>/dev/null </dev/null)
	if usable "$py"; then
		tmp=$cache.$$.tmp
		if [ ! -d "$cache" ] && mkdir -p "$CLAUDE_PLUGIN_DATA" 2>/dev/null && printf '%s\n' "$py" >"$tmp" 2>/dev/null; then
			mv -f "$tmp" "$cache" 2>/dev/null || rm -f "$tmp" 2>/dev/null
		else
			rm -f "$tmp" 2>/dev/null
		fi
		exec "$py" -I "$hook" "$@"
	fi
fi
exec python3 -I "$hook" "$@"
