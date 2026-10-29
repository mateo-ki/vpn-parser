#!/usr/bin/env bash
#
# make_apk.sh - OpenWrt 25.x+ .apk package generator for vpn-parser
# Usage: ./make_apk.sh <binary_path> <arch> <output_dir> [version]
# Example: ./make_apk.sh target/x86_64-unknown-linux-musl/release/vpn-parser x86_64 dist 1.8.0
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BINARY_PATH="${1:-}"
APK_ARCH="${2:-x86_64}"
OUTPUT_DIR="${3:-dist}"
VERSION="${4:-}"

if [ -z "$BINARY_PATH" ] || [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at '$BINARY_PATH'" >&2
    echo "Usage: $0 <binary_path> <arch> <output_dir> [version]" >&2
    exit 1
fi

if [ -z "$VERSION" ]; then
    VERSION=$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | cut -d '"' -f2)
fi

echo "==> Packaging vpn-parser v${VERSION} for OpenWrt 25.x (Architecture: $APK_ARCH)..."

# Ensure apk with mkpkg is available
# 注意：不能用 `apk --help | grep -q mkpkg`——脚本开了 pipefail，grep -q 命中即退出，
# apk 继续写 help 触发 SIGPIPE(141)，管道整体被判失败，探测永远不通过（CI 一直
# 因此静默跳过 .apk 打包）。先捕获输出再做模式匹配，不吃 SIGPIPE。
check_apk() {
    local bin="$1"
    local help_out=""
    [ -n "$bin" ] && [ -x "$bin" ] || return 1
    "$bin" --version >/dev/null 2>&1 || return 1
    if "$bin" mkpkg --help >/dev/null 2>&1; then
        return 0
    fi
    help_out="$("$bin" --help 2>&1)" || true
    [[ "$help_out" == *mkpkg* ]]
}

APK_BIN=""
if command -v apk >/dev/null 2>&1 && check_apk "$(which apk)"; then
    APK_BIN="$(which apk)"
elif check_apk "/usr/local/bin/apk"; then
    APK_BIN="/usr/local/bin/apk"
elif check_apk "$HOME/.local/bin/apk"; then
    APK_BIN="$HOME/.local/bin/apk"
elif [ -f "$SCRIPT_DIR/install_apk_tools.sh" ]; then
    echo "==> apk command with mkpkg not detected. Attempting to install via install_apk_tools.sh..."
    bash "$SCRIPT_DIR/install_apk_tools.sh" || true
    if command -v apk >/dev/null 2>&1 && check_apk "$(which apk)"; then
        APK_BIN="$(which apk)"
    elif check_apk "/usr/local/bin/apk"; then
        APK_BIN="/usr/local/bin/apk"
    elif check_apk "$HOME/.local/bin/apk"; then
        APK_BIN="$HOME/.local/bin/apk"
    fi
fi

if [ -z "$APK_BIN" ]; then
    echo "Warning: 'apk' command with 'mkpkg' subcommand is not available. Skipping .apk generation." >&2
    exit 0
fi

echo "==> Using apk-tools binary: $APK_BIN"

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
BUILD_DIR="$(mktemp -d -t vpn-parser-apk-XXXXXX)"
trap 'rm -rf "$BUILD_DIR"' EXIT

# 1. Prepare file system root hierarchy
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

# 2. Prepare maintainer scripts
POST_INSTALL="$SCRIPT_DIR/apk/post-install"
POST_UPGRADE="$SCRIPT_DIR/apk/post-upgrade"
PRE_DEINSTALL="$SCRIPT_DIR/apk/pre-deinstall"

chmod +x "$POST_INSTALL" "$POST_UPGRADE" "$PRE_DEINSTALL"

# 3. Create .apk package via apk mkpkg
APK_NAME="vpn-parser_${VERSION}_${APK_ARCH}.apk"
APK_PATH="$OUTPUT_DIR/$APK_NAME"

"$APK_BIN" mkpkg \
    --info "name:vpn-parser" \
    --info "version:$VERSION" \
    --info "description:VPN Parser is a lightweight, headless subscription collector, speedtester, and node quality management microservice for OpenWrt 25.x." \
    --info "arch:$APK_ARCH" \
    --info "license:MIT" \
    --info "origin:vpn-parser" \
    --info "url:https://github.com/mateo-ki/vpn-parser" \
    --info "maintainer:mateo-ki <https://github.com/mateo-ki/vpn-parser>" \
    --info "depends:libc,ca-certificates" \
    --script "post-install:$POST_INSTALL" \
    --script "post-upgrade:$POST_UPGRADE" \
    --script "pre-deinstall:$PRE_DEINSTALL" \
    --files "$DATA_DIR" \
    --output "$APK_PATH"

echo "==> Successfully created OpenWrt 25.x package: $APK_PATH"

# 4. Generate SHA-256 Checksum
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUTPUT_DIR" && sha256sum "$APK_NAME" > "${APK_NAME}.sha256")
    echo "==> Checksum: $(cat "$OUTPUT_DIR/${APK_NAME}.sha256")"
fi
