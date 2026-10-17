#!/usr/bin/env bash
#
# make_ipk.sh - Lightweight OpenWrt .ipk package generator for vpn-parser
# Usage: ./make_ipk.sh <binary_path> <opkg_arch> <output_dir> [version]
# Example: ./make_ipk.sh target/x86_64-unknown-linux-musl/release/vpn-parser x86_64 dist 1.8.0

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BINARY_PATH="${1:-}"
OPKG_ARCH="${2:-x86_64}"
OUTPUT_DIR="${3:-dist}"
VERSION="${4:-}"

if [ -z "$BINARY_PATH" ] || [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at '$BINARY_PATH'" >&2
    echo "Usage: $0 <binary_path> <opkg_arch> <output_dir> [version]" >&2
    exit 1
fi

if [ -z "$VERSION" ]; then
    VERSION=$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | cut -d '"' -f2)
fi

echo "==> Packaging vpn-parser v${VERSION} for OpenWrt ($OPKG_ARCH)..."

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
BUILD_DIR="$(mktemp -d -t vpn-parser-ipk-XXXXXX)"
trap 'rm -rf "$BUILD_DIR"' EXIT

# 1. Prepare data directory
DATA_DIR="$BUILD_DIR/data"
mkdir -p "$DATA_DIR/usr/bin"
mkdir -p "$DATA_DIR/etc/init.d"
mkdir -p "$DATA_DIR/etc/config"

cp "$BINARY_PATH" "$DATA_DIR/usr/bin/vpn-parser"
chmod 755 "$DATA_DIR/usr/bin/vpn-parser"

# Strip binary if strip is available
if command -v strip >/dev/null 2>&1; then
    strip --strip-unneeded "$DATA_DIR/usr/bin/vpn-parser" 2>/dev/null || true
fi

cp "$SCRIPT_DIR/files/etc/init.d/vpn-parser" "$DATA_DIR/etc/init.d/vpn-parser"
chmod 755 "$DATA_DIR/etc/init.d/vpn-parser"

cp "$SCRIPT_DIR/files/etc/config/vpn-parser" "$DATA_DIR/etc/config/vpn-parser"
chmod 644 "$DATA_DIR/etc/config/vpn-parser"

# Pack data.tar.gz
(
    cd "$DATA_DIR"
    tar --owner=0 --group=0 -czf "$BUILD_DIR/data.tar.gz" .
)

# 2. Prepare control directory
CONTROL_DIR="$BUILD_DIR/control"
mkdir -p "$CONTROL_DIR"

sed -e "s/__VERSION__/$VERSION/g" \
    -e "s/__ARCH__/$OPKG_ARCH/g" \
    "$SCRIPT_DIR/control/control" > "$CONTROL_DIR/control"
chmod 644 "$CONTROL_DIR/control"

cp "$SCRIPT_DIR/control/postinst" "$CONTROL_DIR/postinst"
chmod 755 "$CONTROL_DIR/postinst"

cp "$SCRIPT_DIR/control/prerm" "$CONTROL_DIR/prerm"
chmod 755 "$CONTROL_DIR/prerm"

# Pack control.tar.gz
(
    cd "$CONTROL_DIR"
    tar --owner=0 --group=0 -czf "$BUILD_DIR/control.tar.gz" .
)

# 3. Create debian-binary
echo "2.0" > "$BUILD_DIR/debian-binary"

# 4. Create final .ipk archive
IPK_NAME="vpn-parser_${VERSION}_${OPKG_ARCH}.ipk"
IPK_PATH="$OUTPUT_DIR/$IPK_NAME"

(
    cd "$BUILD_DIR"
    # In opkg packages, debian-binary must precede data and control
    tar --owner=0 --group=0 -czf "$IPK_PATH" debian-binary data.tar.gz control.tar.gz
)

echo "==> Successfully created: $IPK_PATH"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUTPUT_DIR" && sha256sum "$IPK_NAME")
fi
