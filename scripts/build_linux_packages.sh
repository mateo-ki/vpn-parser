#!/usr/bin/env bash
# ==============================================================================
# build_linux_packages.sh - 构建 Linux release 包 (AppImage, deb, rpm)
# ==============================================================================
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK_DIR="$HOME/vpn-parser"
DIST_DIR="$SRC_DIR/dist"
mkdir -p "$DIST_DIR"

echo "=== [1/6] 同步源码至 WSL 工作区 ==="
mkdir -p "$WORK_DIR"
tar -C "$SRC_DIR" -cf - \
    --exclude=./target \
    --exclude=./dist \
    --exclude=./data \
    --exclude=./.git \
    --exclude=./.cache \
    --exclude=./logs \
    --exclude=./design . 2>/dev/null | tar -C "$WORK_DIR" -xf -
cd "$WORK_DIR"

# 确保脚本权限
chmod 755 packaging/linux/deb/postinst packaging/linux/deb/postrm 2>/dev/null || true

# 读取 Cargo.toml 中的版本号
VERSION=$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1)
echo "当前构建版本: $VERSION"

echo "=== [2/6] 编译 Release 可执行文件 ==="
cargo build --release --bin vpn-parser
BINARY="$WORK_DIR/target/release/vpn-parser"
if [ ! -f "$BINARY" ]; then
    echo "错误: 未找到编译产物 $BINARY" >&2
    exit 1
fi
strip "$BINARY" || true

echo "=== [3/6] 构建 deb 安装包 ==="
rm -f target/debian/*.deb
cargo deb
DEB_FILE=$(ls target/debian/*.deb | head -1)
echo "已生成 deb: $(basename "$DEB_FILE") ($(du -h "$DEB_FILE" | cut -f1))"

echo "=== [4/6] 构建 rpm 安装包 ==="
rm -f target/generate-rpm/*.rpm
cargo generate-rpm
RPM_FILE=$(ls target/generate-rpm/*.rpm | head -1)
echo "已生成 rpm: $(basename "$RPM_FILE") ($(du -h "$RPM_FILE" | cut -f1))"

echo "=== [5/6] 构建 AppImage 便携包 ==="
TOOL_DIR="$HOME/.cache/appimagetool"
mkdir -p "$TOOL_DIR"
if [ ! -f "$TOOL_DIR/squashfs-root/AppRun" ]; then
    SRC_TOOL="$SRC_DIR/.cache/appimagetool-x86_64.AppImage"
    if [ -f "$SRC_TOOL" ]; then
        echo "使用本地缓存的 appimagetool..."
        cp "$SRC_TOOL" "$TOOL_DIR/appimagetool.AppImage"
    else
        echo "下载官方 appimagetool..."
        curl -fSL "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage" -o "$TOOL_DIR/appimagetool.AppImage"
    fi
    chmod +x "$TOOL_DIR/appimagetool.AppImage"
    (cd "$TOOL_DIR" && ./appimagetool.AppImage --appimage-extract && rm -f ./appimagetool.AppImage)
fi
APPIMAGETOOL="$TOOL_DIR/squashfs-root/AppRun"

# 检查 AppImage runtime 文件
RUNTIME_FILE="$TOOL_DIR/runtime-x86_64"
if [ ! -f "$RUNTIME_FILE" ]; then
    SRC_RUNTIME="$SRC_DIR/.cache/runtime-x86_64"
    if [ -f "$SRC_RUNTIME" ]; then
        echo "使用本地缓存的 runtime-x86_64..."
        cp "$SRC_RUNTIME" "$RUNTIME_FILE"
    else
        echo "下载官方 runtime-x86_64..."
        curl -fSL "https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64" -o "$RUNTIME_FILE"
    fi
fi
chmod +x "$RUNTIME_FILE"

# 构造 AppDir 目录结构
APPDIR="$WORK_DIR/target/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/16x16/apps"
mkdir -p "$APPDIR/usr/share/icons/hicolor/32x32/apps"
mkdir -p "$APPDIR/usr/share/icons/hicolor/48x48/apps"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"
mkdir -p "$APPDIR/usr/share/doc/vpn-parser"

# 复制二进制和元数据
cp "$BINARY" "$APPDIR/usr/bin/vpn-parser"
cp packaging/linux/vpn-parser.desktop "$APPDIR/usr/share/applications/vpn-parser.desktop"
cp packaging/linux/vpn-parser.desktop "$APPDIR/vpn-parser.desktop"
cp assets/app-icon-16.png "$APPDIR/usr/share/icons/hicolor/16x16/apps/vpn-parser.png"
cp assets/app-icon-32.png "$APPDIR/usr/share/icons/hicolor/32x32/apps/vpn-parser.png"
cp assets/app-icon-48.png "$APPDIR/usr/share/icons/hicolor/48x48/apps/vpn-parser.png"
cp assets/app-icon-256.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/vpn-parser.png"
cp assets/app-icon-256.png "$APPDIR/vpn-parser.png"
(cd "$APPDIR" && ln -sf vpn-parser.png .DirIcon)
cp LICENSE "$APPDIR/usr/share/doc/vpn-parser/" 2>/dev/null || true
cp README.md "$APPDIR/usr/share/doc/vpn-parser/" 2>/dev/null || true

# 制作启动器 AppRun
cat > "$APPDIR/AppRun" << 'EOF'
#!/bin/sh
set -e
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib:${HERE}/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH}"
export XDG_DATA_DIRS="${HERE}/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
exec "${HERE}/usr/bin/vpn-parser" "$@"
EOF
chmod 755 "$APPDIR/AppRun"

# 打包生成 AppImage
mkdir -p "$WORK_DIR/target/appimage"
APPIMAGE_OUTPUT="$WORK_DIR/target/appimage/vpn-parser-${VERSION}-x86_64.AppImage"
rm -f "$APPIMAGE_OUTPUT"

echo "正在执行 appimagetool 打包..."
ARCH=x86_64 "$APPIMAGETOOL" --no-appstream --runtime-file "$RUNTIME_FILE" "$APPDIR" "$APPIMAGE_OUTPUT"
chmod +x "$APPIMAGE_OUTPUT"
echo "已生成 AppImage: $(basename "$APPIMAGE_OUTPUT") ($(du -h "$APPIMAGE_OUTPUT" | cut -f1))"

echo "=== [6/6] 汇总并复制所有发布包至 dist/ ==="
cp "$DEB_FILE" "$DIST_DIR/"
cp "$RPM_FILE" "$DIST_DIR/"
cp "$APPIMAGE_OUTPUT" "$DIST_DIR/"

echo ""
echo "========================================================================"
echo "Linux 发布包构建成功！输出至 dist/ 目录："
echo "========================================================================"
ls -lh "$DIST_DIR"/*${VERSION}*
echo ""
echo "=== SHA-256 校验和 ==="
sha256sum "$DIST_DIR"/vpn-parser*${VERSION}*
