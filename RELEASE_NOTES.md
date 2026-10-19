# 🚀 VPN Parser v1.8.8 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.8**！本次更新重点修复了 **Windows 注册表底层 Connections 二进制块死 PAC 脚本自动净化**、**测试用例与后台任务对宿主机活跃系统代理的保护隔离**，进一步提升系统代理长时间挂载的绝对稳定性！

---

## 🌟 新特性与核心更新

### 🛡️ 1. Windows WinINet 底层二进制配置（Connections）深度净化
- **净化 `DefaultConnectionSettings` 与 `SavedLegacySettings` 二进制块**：
  - Windows 系统代理不仅存放在常规键值中，还会序列化一份二进制数据存放在 `Internet Settings\Connections` 下。此前外部工具（如 v2rayN）遗留的 PAC 脚本地址（`127.0.0.1:10811/pac`）仍固化在二进制块中，导致 Windows 或其他程序重新读取连接设置时因尝试加载死 PAC 脚本失败而自动触发操作系统底层回滚将 `ProxyEnable` 关停。
  - 本次更新在启用/恢复系统代理时，底层直接将 Connections 二进制块中的 PAC URL 长度置零并同步修正状态标志，彻底阻断了 WinINet 尝试回连 10811 端口的路径。

---

### 🔒 2. 开发与运行期宿主机系统代理现场保护
- **单元测试注册表现场还原**：
  - 修复 `test_direct_disable_wininet_proxy_atomic_executes_safely` 与 `test_self_heal_stale_proxy_inactive_or_safe` 单元测试在运行时修改宿主机 `ProxyEnable` 注册表并未还原的问题，确保测试与构建过程绝不干扰前台客户端正常上网。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.8_*.ipk ./luci-app-vpn-parser_1.8.8_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

---

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.8_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.8_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.8_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.8_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.8_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.8-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.8-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.8-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.8-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.8-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.8-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.8-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.7...v1.8.8
