#!/usr/bin/env bash
#
# make_luci_apk.sh - OpenWrt 25.x+ LuCI .apk package generator for luci-app-vpn-parser
# Usage: ./make_luci_apk.sh [output_dir] [version]
# Example: ./make_luci_apk.sh dist 1.8.0
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

OUTPUT_DIR="${1:-dist}"
VERSION="${2:-}"

if [ -z "$VERSION" ]; then
    VERSION=$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | cut -d '"' -f2)
fi

echo "==> Packaging luci-app-vpn-parser v${VERSION} for OpenWrt 25.x (Architecture: noarch / all)..."

# Ensure apk with mkpkg is available
APK_BIN=""
if command -v apk >/dev/null 2>&1 && (apk mkpkg --help >/dev/null 2>&1 || apk --help 2>&1 | grep -q "mkpkg"); then
    APK_BIN="$(which apk)"
elif [ -x "/usr/local/bin/apk" ]; then
    APK_BIN="/usr/local/bin/apk"
elif [ -x "$HOME/.local/bin/apk" ]; then
    APK_BIN="$HOME/.local/bin/apk"
elif [ -f "$REPO_ROOT/packaging/openwrt/install_apk_tools.sh" ]; then
    echo "==> apk command with mkpkg not detected. Attempting to install via install_apk_tools.sh..."
    bash "$REPO_ROOT/packaging/openwrt/install_apk_tools.sh"
    if command -v apk >/dev/null 2>&1; then
        APK_BIN="$(which apk)"
    elif [ -x "/usr/local/bin/apk" ]; then
        APK_BIN="/usr/local/bin/apk"
    elif [ -x "$HOME/.local/bin/apk" ]; then
        APK_BIN="$HOME/.local/bin/apk"
    fi
fi

if [ -z "$APK_BIN" ]; then
    echo "Error: 'apk' command with 'mkpkg' subcommand is required to build OpenWrt 25.x .apk packages." >&2
    echo "Please run './packaging/openwrt/install_apk_tools.sh' or install apk-tools v3.0+." >&2
    exit 1
fi

echo "==> Using apk-tools binary: $APK_BIN"

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
BUILD_DIR="$(mktemp -d -t luci-app-vpn-parser-apk-XXXXXX)"
trap 'rm -rf "$BUILD_DIR"' EXIT

# 1. Prepare data directory hierarchy
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

# 2. Prepare maintainer scripts
POST_INSTALL="$SCRIPT_DIR/apk/post-install"
POST_UPGRADE="$SCRIPT_DIR/apk/post-upgrade"
PRE_DEINSTALL="$SCRIPT_DIR/apk/pre-deinstall"

chmod +x "$POST_INSTALL" "$POST_UPGRADE" "$PRE_DEINSTALL"

# 3. Create .apk package via apk mkpkg
APK_NAME="luci-app-vpn-parser_${VERSION}_all.apk"
APK_PATH="$OUTPUT_DIR/$APK_NAME"

"$APK_BIN" mkpkg \
    --info "name:luci-app-vpn-parser" \
    --info "version:$VERSION" \
    --info "description:LuCI Support for VPN Parser microservice on OpenWrt 25.x" \
    --info "arch:noarch" \
    --info "license:MIT" \
    --info "origin:luci-app-vpn-parser" \
    --info "url:https://github.com/mateo-ki/vpn-parser" \
    --info "maintainer:mateo-ki <https://github.com/mateo-ki/vpn-parser>" \
    --info "depends:vpn-parser,luci-base,rpcd" \
    --script "post-install:$POST_INSTALL" \
    --script "post-upgrade:$POST_UPGRADE" \
    --script "pre-deinstall:$PRE_DEINSTALL" \
    --files "$DATA_DIR" \
    --output "$APK_PATH"

echo "==> Successfully created OpenWrt 25.x LuCI package: $APK_PATH"

# 4. Generate SHA-256 Checksum
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUTPUT_DIR" && sha256sum "$APK_NAME" > "${APK_NAME}.sha256")
    echo "==> Checksum: $(cat "$OUTPUT_DIR/${APK_NAME}.sha256")"
fi
