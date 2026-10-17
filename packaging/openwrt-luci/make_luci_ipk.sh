#!/usr/bin/env bash
#
# make_luci_ipk.sh - Lightweight OpenWrt LuCI .ipk package generator for luci-app-vpn-parser
# Usage: ./make_luci_ipk.sh [output_dir] [version]
# Example: ./make_luci_ipk.sh dist 1.8.0
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

OUTPUT_DIR="${1:-dist}"
VERSION="${2:-}"

if [ -z "$VERSION" ]; then
    VERSION=$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | cut -d '"' -f2)
fi

echo "==> Packaging luci-app-vpn-parser v${VERSION} for OpenWrt (Architecture: all)..."

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
BUILD_DIR="$(mktemp -d -t luci-app-vpn-parser-ipk-XXXXXX)"
trap 'rm -rf "$BUILD_DIR"' EXIT

# 1. Prepare data directory
DATA_DIR="$BUILD_DIR/data"
mkdir -p "$DATA_DIR/www/luci-static/resources/view/vpn-parser"
mkdir -p "$DATA_DIR/usr/share/luci/menu.d"
mkdir -p "$DATA_DIR/usr/share/rpcd/acl.d"
mkdir -p "$DATA_DIR/usr/libexec/rpcd"
mkdir -p "$DATA_DIR/etc/uci-defaults"

# Copy web view assets to /www/luci-static/...
cp "$SCRIPT_DIR"/htdocs/luci-static/resources/view/vpn-parser/*.js "$DATA_DIR/www/luci-static/resources/view/vpn-parser/"
chmod 644 "$DATA_DIR/www/luci-static/resources/view/vpn-parser/"*.js

# Copy menu and ACL definitions
cp "$SCRIPT_DIR/root/usr/share/luci/menu.d/luci-app-vpn-parser.json" "$DATA_DIR/usr/share/luci/menu.d/"
chmod 644 "$DATA_DIR/usr/share/luci/menu.d/luci-app-vpn-parser.json"

cp "$SCRIPT_DIR/root/usr/share/rpcd/acl.d/luci-app-vpn-parser.json" "$DATA_DIR/usr/share/rpcd/acl.d/"
chmod 644 "$DATA_DIR/usr/share/rpcd/acl.d/luci-app-vpn-parser.json"

# Copy rpcd ubus provider
cp "$SCRIPT_DIR/root/usr/libexec/rpcd/luci.vpn-parser" "$DATA_DIR/usr/libexec/rpcd/"
chmod 755 "$DATA_DIR/usr/libexec/rpcd/luci.vpn-parser"

# Copy uci-defaults script
cp "$SCRIPT_DIR/root/etc/uci-defaults/80_luci-app-vpn-parser" "$DATA_DIR/etc/uci-defaults/"
chmod 755 "$DATA_DIR/etc/uci-defaults/80_luci-app-vpn-parser"

# Pack data.tar.gz
(
    cd "$DATA_DIR"
    tar --owner=0 --group=0 -czf "$BUILD_DIR/data.tar.gz" .
)

# 2. Prepare control directory
CONTROL_DIR="$BUILD_DIR/control"
mkdir -p "$CONTROL_DIR"

sed -e "s/__VERSION__/$VERSION/g" \
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
IPK_NAME="luci-app-vpn-parser_${VERSION}_all.ipk"
IPK_PATH="$OUTPUT_DIR/$IPK_NAME"

(
    cd "$BUILD_DIR"
    tar --owner=0 --group=0 -czf "$IPK_PATH" debian-binary data.tar.gz control.tar.gz
)

echo "==> Successfully created: $IPK_PATH"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUTPUT_DIR" && sha256sum "$IPK_NAME" > "$IPK_NAME.sha256")
    echo "==> Checksum: $(cat "$OUTPUT_DIR/$IPK_NAME.sha256")"
fi
