#!/bin/sh
# Write Formula/raylauncher.rb for a published release into a local homebrew-tap checkout.
# It changes no git state: review the diff, then commit and push yourself.
#
#   scripts/update-formula.sh v0.1.0 ../homebrew-tap
#
# Environment:
#   RAYLAUNCHER_REPO      GitHub repo the release lives in (default: undo-git-pull/raylauncher)
#   RAYLAUNCHER_BASE_URL  download folder override, e.g. file:///path/to/dist for testing
set -eu

usage="usage: $0 <tag, e.g. v0.1.0> <path to homebrew-tap checkout>"
tag="${1:?$usage}"
tap="${2:?$usage}"
repo="${RAYLAUNCHER_REPO:-undo-git-pull/raylauncher}"
asset="raylauncher-macos-universal.tar.gz"
version="${tag#v}"
root="$(cd "$(dirname "$0")/.." && pwd)"
base="${RAYLAUNCHER_BASE_URL:-https://github.com/$repo/releases/download/$tag}"

[ -d "$tap/.git" ] || { echo "$tap is not a git checkout" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $tag release files from $repo"
curl -fsSL "$base/$asset" -o "$tmp/$asset"
curl -fsSL "$base/$asset.sha256" -o "$tmp/$asset.sha256"

# The formula pins the checksum, so confirm the published one matches the actual file.
published="$(cut -d' ' -f1 "$tmp/$asset.sha256")"
actual="$(shasum -a 256 "$tmp/$asset" | cut -d' ' -f1)"
if [ "$published" != "$actual" ]; then
  echo "Checksum mismatch: published $published, archive is $actual" >&2
  exit 1
fi

out="$tap/Formula/raylauncher.rb"
mkdir -p "$tap/Formula"
# Drop the template's header comment first, then fill in the placeholders.
sed -e '/^# Template for/,/^# .* and writes the result/d' \
  -e "s|@VERSION@|$version|g" -e "s|@SHA256@|$actual|g" -e "s|@REPO@|$repo|g" \
  "$root/packaging/homebrew/raylauncher.rb.in" > "$out"

grep -q '^class Raylauncher < Formula' "$out" || { echo "rendered formula is malformed" >&2; exit 1; }
if grep -q '@[A-Z0-9]*@' "$out"; then echo "unfilled placeholder in $out" >&2; exit 1; fi
if command -v ruby >/dev/null; then ruby -c "$out" >/dev/null; fi

echo "Wrote $out ($version, sha256 $actual)"
echo
echo "Next:"
echo "  cd $tap && git diff"
echo "  git add Formula/raylauncher.rb && git commit -m 'raylauncher $version' && git push"
