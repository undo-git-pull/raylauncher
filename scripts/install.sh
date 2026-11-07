#!/bin/sh
# Install raylauncher from GitHub Releases into ~/.local/bin and start it at login.
#
#   curl -fsSL https://github.com/undo-git-pull/raylauncher/releases/latest/download/install.sh | sh
#
# Environment:
#   RAYLAUNCHER_VERSION  release tag to install, e.g. v0.1.0 (default: latest)
#   RAYLAUNCHER_REPO     GitHub repo to download from (default: undo-git-pull/raylauncher)
set -eu

REPO="${RAYLAUNCHER_REPO:-undo-git-pull/raylauncher}"
VERSION="${RAYLAUNCHER_VERSION:-latest}"
ASSET="raylauncher-macos-universal.tar.gz"
BIN_DIR="$HOME/.local/bin"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "raylauncher runs on macOS only." >&2
  exit 1
fi

if [ "$VERSION" = "latest" ]; then
  base="https://github.com/$REPO/releases/latest/download"
else
  base="https://github.com/$REPO/releases/download/$VERSION"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $ASSET ($VERSION) from $REPO"
curl -fsSL "$base/$ASSET" -o "$tmp/$ASSET"
curl -fsSL "$base/$ASSET.sha256" -o "$tmp/$ASSET.sha256"
(cd "$tmp" && shasum -a 256 -c "$ASSET.sha256" >/dev/null) || {
  echo "Checksum mismatch for $ASSET; not installing." >&2
  exit 1
}
tar -xzf "$tmp/$ASSET" -C "$tmp"

mkdir -p "$BIN_DIR"
# Remove before copying: overwriting a binary in place can invalidate its code signature.
rm -f "$BIN_DIR/raylauncher"
cp "$tmp/raylauncher" "$BIN_DIR/raylauncher"
chmod 755 "$BIN_DIR/raylauncher"
xattr -d com.apple.quarantine "$BIN_DIR/raylauncher" 2>/dev/null || true

"$BIN_DIR/raylauncher" install

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "Note: $BIN_DIR is not on your PATH. Add it to run 'raylauncher' commands by name." ;;
esac
echo "Done. Press Option+Space to open raylauncher. Config: ~/.config/raylauncher"
