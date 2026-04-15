#!/usr/bin/env bash
set -euo pipefail

# Installs lnd + lncli into ./bin for local testnet4 workflows.
# No Homebrew/system package manager required.
#
# Optional env:
#   LND_VERSION=v0.20.1-beta (if unset, script fetches latest release tag)

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT_DIR/bin"
TMP_DIR="$ROOT_DIR/.tmp"

mkdir -p "$BIN_DIR" "$TMP_DIR"

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin) os="darwin" ;;
    Linux) os="linux" ;;
    *)
      echo "Unsupported OS: $os" >&2
      exit 1
      ;;
  esac

  case "$arch" in
    arm64|aarch64) arch="arm64" ;;
    x86_64|amd64) arch="amd64" ;;
    *)
      echo "Unsupported architecture: $arch" >&2
      exit 1
      ;;
  esac

  printf "%s-%s\n" "$os" "$arch"
}

resolve_latest_tag() {
  curl -fsSL "https://api.github.com/repos/lightningnetwork/lnd/releases/latest" \
    | node -e '
      let d = "";
      process.stdin.on("data", (c) => (d += c));
      process.stdin.on("end", () => {
        const j = JSON.parse(d);
        if (!j.tag_name) {
          process.stderr.write("Could not resolve latest lnd tag\n");
          process.exit(1);
        }
        process.stdout.write(j.tag_name);
      });
    '
}

resolve_asset_url() {
  local version="$1"
  local platform="$2"
  curl -fsSL "https://api.github.com/repos/lightningnetwork/lnd/releases/tags/$version" \
    | node -e '
      const platform = process.argv[1];
      let d = "";
      process.stdin.on("data", (c) => (d += c));
      process.stdin.on("end", () => {
        const j = JSON.parse(d);
        const assets = Array.isArray(j.assets) ? j.assets : [];
        const re = new RegExp(`lnd-${platform}-.*\\.tar\\.gz$`);
        const match = assets.find((a) => re.test(a.name));
        if (!match || !match.browser_download_url) {
          process.stderr.write(`No matching asset found for platform ${platform}\n`);
          process.exit(1);
        }
        process.stdout.write(match.browser_download_url);
      });
    ' "$platform"
}

PLATFORM="$(detect_platform)"
VERSION="${LND_VERSION:-$(resolve_latest_tag)}"
ASSET_URL="$(resolve_asset_url "$VERSION" "$PLATFORM")"
ARCHIVE_PATH="$TMP_DIR/$(basename "$ASSET_URL")"
EXTRACT_DIR="$TMP_DIR/lnd-extract-$VERSION-$PLATFORM"

echo "Installing lnd $VERSION for $PLATFORM"
echo "Asset: $ASSET_URL"

rm -rf "$EXTRACT_DIR"
mkdir -p "$EXTRACT_DIR"

curl -fL "$ASSET_URL" -o "$ARCHIVE_PATH"
tar -xzf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"

PACKAGE_DIR=""
for candidate in "$EXTRACT_DIR"/lnd-*; do
  if [[ -d "$candidate" ]]; then
    PACKAGE_DIR="$candidate"
    break
  fi
done
[[ -n "$PACKAGE_DIR" ]] || {
  echo "Failed to locate extracted lnd package directory" >&2
  exit 1
}

install -m 0755 "$PACKAGE_DIR/lnd" "$BIN_DIR/lnd"
install -m 0755 "$PACKAGE_DIR/lncli" "$BIN_DIR/lncli"

echo
echo "Installed:"
"$BIN_DIR/lnd" --version
"$BIN_DIR/lncli" --version
echo
echo "Binaries:"
echo "  $BIN_DIR/lnd"
echo "  $BIN_DIR/lncli"
