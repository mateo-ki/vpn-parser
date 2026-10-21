# 🚀 VPN Parser v1.8.10 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.10**！本次更新修好了 **Windows TUN 模式点了没反应、只回一句"需要管理员权限"** 的问题：提权交接这条链现在真的接通了 —— 程序会把当前配置交给一个以管理员身份启动的实例接管，本窗口随后自动退出。

---

## 🌟 新特性与核心更新

### 🔓 1. Windows TUN 模式提权交接接通
- **修掉"TUN 模式启动不了"**：
  - 这条链此前只有消费端：提权实例读 `--elevated-proxy-config`、轮询 3 秒接管单实例互斥量、命令行去重都在，但**没有任何地方调用 `request_elevation()`，也没有人生成那段提权配置**。于是在非提权实例里点「启动代理」，界面只把状态文案改成"正在请求管理员权限…"，随后照样调用内核启动，被内核原样打回 `TUN 模式需要管理员权限`。
  - 现在：TUN + 未提权 → 生成 `--elevated-proxy-config`（当前分流配置的 base64，URL-safe 无填充）+ `--start-proxy-node`（要起的节点），交给 `runas` 起来的提权实例；本实例随即**真退出**（绕开"关到托盘"）释放互斥量，提权实例那边接管并把 TUN 内核起好。
  - 状态文案跟着说实话：以前那句"以无头模式启动"是假话（提权实例是带窗口的），现在写的是"提权实例会带着 TUN 内核启动，这个窗口随后关闭"。
  - 实测：非提权实例点启动后，父实例日志 `TUN 交接：已请求管理员权限，本实例退出以释放单实例互斥量`，提权实例 `Acquired single instance mutex on attempt 2.`（约 110ms 后接管）。

---

### 🧯 2. 交接参数的回归闸（防"一闪就没"）
- **提权实例的命令行只能有一份自己的 flag**：`--start-proxy-node` 出现两次时 clap 直接 `ArgumentConflict` 并 `exit(2)`，提权实例在写下第一行日志之前就死掉（v1.8.6 那次现象：点 TUN 启动，窗口一闪就没）。本轮把生产端接上后，补了这条反向对照：本进程自己就是带着 `--start-proxy-node` 起来的（安装器、开机自启、以及上一次交接都会这样）时，拼完仍只剩一个该 flag，且值是新节点。
- **配置编解码合并成一份实现**：`encode_elevated_proxy_config` / `decode_elevated_proxy_config` 放在同一处，读写两侧不再各写一份；坏 payload 必须报错，绝不静默回落成默认配置（那会让提权实例用一套没人写过的选项起内核）。`controller_secret` 是一次性密钥（`serde(skip)`），不跨进程带，提权实例自己生成一份。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.10_*.ipk ./luci-app-vpn-parser_1.8.10_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

---

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.10_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.10_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.10_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.10_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.10_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.10-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.10-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.10-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.10-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.10-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.10-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.10-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.9...v1.8.10
