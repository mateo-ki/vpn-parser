# 🚀 VPN Parser v1.8.7 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.7**！本次更新重点带来了**长连接稳定性与防频繁断线深度调优**、**Windows 系统代理死 PAC 残留净化与多客户端共存保护**、**macOS 中文字体基线对齐校正**，以及 **OpenWrt apk-tools 静态编译与全平台 CI 强化**！

---

## 🌟 新特性与核心更新

### 🛡️ 1. 长连接稳定性与防频繁断线调优
- **软化断路器熔断机制**：
  - 将容错阈值（`breaker_failures`）提升至 4 次，适应跨国跨洋网络在高峰期的丢包抖动，避免 1~2 次偶发丢包直接触发断路器掐断活跃 TCP 连接；
- **放宽探活握手超时上限**：
  - 将端到端握手超时上限由 1500ms 调整至 3500ms，为真实海外网页与延迟较高的远程服务器提供充足缓冲；
- **延长异常重试退避间隔**：
  - 探活异常后的快速复检间隔从 5 秒放宽到 10 秒，消除频繁重试造成的网络拥塞与频繁跳变；
- **修复真实节点伪 TCP 过滤误丢弃**：
  - 标准与深度测速模式下，遇到国内直连阻断纯 TCP Ping 的有效节点不再过早丢弃，交由内核发起真实的 TLS 1.3 / 协议握手，大幅提高可用节点检出率。

---

### 🌐 2. Windows 系统代理防冲突与死 PAC 残留清理
- **自动清理残留 AutoConfigURL (PAC)**：
  - 启动系统代理时自动清理注册表中的历史残留 PAC 脚本地址（如 v2rayN 历史遗留端口），彻底解决 Windows WinINet 因无法加载死 PAC 脚本将 `ProxyEnable` 自动复位为 0 导致浏览器断网的顽疾；
- **优化系统代理看门狗核对机制**：
  - 明确核对机制与自动写回逻辑，提供一键写回保护，避免多工具共存或系统休眠唤醒时的注册表篡改。

---

### 🔤 3. macOS 界面字体基线垂直对齐校正
- **修复按钮文本偏下错位**：
  - 优先选用 Hiragino Sans GB 字体，并针对 PingFang 字体基线高度差异应用 `FontTweak { y_offset_factor: -0.18 }` 补偿校正，彻底消除 macOS 下按钮文本垂直不居中的问题。

---

### 📦 4. OpenWrt 25.x 打包与全平台 CI 强化
- **apk-tools v3 纯静态编译**：
  - OpenWrt apk 打包工具链采用 `-Ddefault_library=static` 静态编译，消除动态链接 `libapk.so.3.0.0` 依赖问题；
- **升级 Windows CI 测试内核**：
  - 更新 Sing-box 至 v1.13.14，全面兼容最新规则配置语法规范。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.7_*.ipk ./luci-app-vpn-parser_1.8.7_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

---

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.7_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.7_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.7_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.7_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.7_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.7-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.7-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.7-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.7-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.7-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.7-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.7-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.6...v1.8.7
