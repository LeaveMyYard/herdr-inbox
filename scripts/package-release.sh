#!/bin/sh
# Package one release target (.github/workflows/release.yml, plan §4 item 2):
#
#   sh scripts/package-release.sh <target> <version> <out dir>
#
# writes <out dir>/herdr-inbox-v<version>-<target>.tar.gz holding `herdr-inbox` (the binary
# from target/<target>/release) and LICENSE, both at the top level, owned by root, with
# fixed modes and times so a rebuild of the same binary packs the same bytes.
set -eu
target=$1
version=$2
out=$3
bin="target/$target/release/herdr-inbox"
[ -x "$bin" ] || { echo "no binary at $bin" >&2; exit 1; }
stage=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/package.XXXXXX")
trap 'rm -rf "$stage"' EXIT
cp "$bin" "$stage/herdr-inbox"
cp LICENSE "$stage/LICENSE"
chmod 0755 "$stage/herdr-inbox"
chmod 0644 "$stage/LICENSE"
mkdir -p "$out"
name="herdr-inbox-v$version-$target.tar.gz"
# GNU tar on Linux, bsdtar on macOS: the reproducibility flags differ
if tar --version 2>/dev/null | grep -q 'GNU tar'; then
	tar -C "$stage" --owner=0 --group=0 --numeric-owner --mtime='@0' --sort=name -cf - herdr-inbox LICENSE | gzip -n >"$out/$name"
else
	touch -t 197001010000 "$stage/herdr-inbox" "$stage/LICENSE"
	tar -C "$stage" --uid 0 --gid 0 -cf - herdr-inbox LICENSE | gzip -n >"$out/$name"
fi
echo "$out/$name"
