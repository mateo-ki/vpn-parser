#!/usr/bin/env bash
#
# install_apk_tools.sh - Install or build apk-tools (v3.0.x with mkpkg support)
# Needed for building OpenWrt 25.x+ .apk packages.
#

set -euo pipefail

if command -v apk >/dev/null 2>&1; then
    if apk mkpkg --help >/dev/null 2>&1 || apk --help 2>&1 | grep -q "mkpkg"; then
        echo "==> apk-tools with mkpkg is already installed: $(which apk)"
        apk --version 2>/dev/null || true
        exit 0
    fi
fi

echo "==> apk-tools with mkpkg not found. Preparing to build apk-tools v3.0.8..."

# Determine installation destination
TARGET_BIN="/usr/local/bin/apk"
if [ ! -w "/usr/local/bin" ] && [ "$(id -u)" -ne 0 ] && ! command -v sudo >/dev/null 2>&1; then
    TARGET_BIN="$HOME/.local/bin/apk"
    mkdir -p "$HOME/.local/bin"
    export PATH="$HOME/.local/bin:$PATH"
fi

TMP_DIR="$(mktemp -d -t apk-tools-build-XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

# Detect package manager on Linux to install dependencies
if command -v apt-get >/dev/null 2>&1; then
    echo "==> Installing build dependencies via apt-get..."
    if [ "$(id -u)" -eq 0 ]; then
        apt-get update -qq
        apt-get install -y -qq meson ninja-build pkg-config pkgconf libssl-dev zlib1g-dev libzstd-dev git gcc
    elif command -v sudo >/dev/null 2>&1; then
        sudo apt-get update -qq
        sudo apt-get install -y -qq meson ninja-build pkg-config pkgconf libssl-dev zlib1g-dev libzstd-dev git gcc
    fi
elif command -v dnf >/dev/null 2>&1; then
    echo "==> Installing build dependencies via dnf..."
    if [ "$(id -u)" -eq 0 ]; then
        dnf install -y -q meson ninja-build pkgconf pkgconf-pkg-config openssl-devel zlib-devel libzstd-devel git gcc
    elif command -v sudo >/dev/null 2>&1; then
        sudo dnf install -y -q meson ninja-build pkgconf pkgconf-pkg-config openssl-devel zlib-devel libzstd-devel git gcc
    fi
elif command -v pacman >/dev/null 2>&1; then
    echo "==> Installing build dependencies via pacman..."
    if [ "$(id -u)" -eq 0 ]; then
        pacman -Sy --noconfirm meson ninja pkgconf openssl zlib zstd git gcc
    elif command -v sudo >/dev/null 2>&1; then
        sudo pacman -Sy --noconfirm meson ninja pkgconf openssl zlib zstd git gcc
    fi
fi

echo "==> Cloning apk-tools v3.0.8..."
git clone --depth 1 --branch v3.0.8 https://github.com/alpinelinux/apk-tools.git "$TMP_DIR/apk-tools"

if ! command -v meson >/dev/null 2>&1; then
    echo "Error: 'meson' build system not found. Please install meson and ninja (e.g., sudo apt install meson ninja-build)." >&2
    exit 1
fi
if ! command -v ninja >/dev/null 2>&1; then
    echo "Error: 'ninja' build tool not found. Please install ninja-build." >&2
    exit 1
fi

echo "==> Configuring with meson..."
meson setup "$TMP_DIR/apk-tools/build" "$TMP_DIR/apk-tools" \
    -Ddocs=disabled \
    -Dlua=disabled \
    -Dhelp=disabled \
    -Dcrypto_backend=openssl

echo "==> Compiling apk..."
ninja -C "$TMP_DIR/apk-tools/build" apk

echo "==> Installing to $TARGET_BIN..."
if [ -w "$(dirname "$TARGET_BIN")" ]; then
    cp "$TMP_DIR/apk-tools/build/src/apk" "$TARGET_BIN"
    chmod 755 "$TARGET_BIN"
elif command -v sudo >/dev/null 2>&1; then
    sudo cp "$TMP_DIR/apk-tools/build/src/apk" "$TARGET_BIN"
    sudo chmod 755 "$TARGET_BIN"
else
    mkdir -p "$HOME/.local/bin"
    cp "$TMP_DIR/apk-tools/build/src/apk" "$HOME/.local/bin/apk"
    chmod 755 "$HOME/.local/bin/apk"
    TARGET_BIN="$HOME/.local/bin/apk"
fi

echo "==> Successfully installed: $TARGET_BIN"
if [ -n "${GITHUB_PATH:-}" ]; then
    echo "$(dirname "$TARGET_BIN")" >> "$GITHUB_PATH"
fi
"$TARGET_BIN" --version 2>/dev/null || true
