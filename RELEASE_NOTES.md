# 🚀 VPN Parser v1.8.11 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.11**！本次更新把**测速改回真正的二阶段淘汰**：TCP 预筛没过的节点不再占用真实连接测试的名额，把"预筛跑满全库却不做任何决定"那段时间收回来，同时让进度条上那个数字重新有意义。

---

## 🌟 新特性与核心更新

### ⚡ 1. 测速二阶段淘汰：先 TCP 过滤，剩下的节点才做真实连接测试
- **修掉的问题**：标准/深度模式下，进入真实握手的条件是 `TCP 通过 || 是公网主机`，而"公网主机"只排除 localhost / 私网 / 保留地址 ⇒ 所有公网域名一律去握手。证据就在库里：16,066 个节点中 **15,329 个的 `latency_test_kind` 是 `google_proxy`** —— TCP 预筛以并发 128、超时 1500ms 跑完整库（约 1.5~3 分钟），却一个都不淘汰，还在进度条上报出一个"成功 10000"这种不该被理解成"可用"的数字。
- **现在的行为**：解出了地址但 TCP 连不上的节点，直接按 `tcp_prefilter` 落库标为不可用（理由写 `connect error` 或 `timeout 1500ms`），**不再进入内核握手**。按上面那个库的量级，握手候选从 16,066 降到约 10,000（**−37%**），一整轮测速随之缩短。
- **保留的例外**：DNS 在我们这一侧压根没解出地址的节点仍然交给内核 —— 内核有自己那条解析路（DoH / 经代理），解析结果可能完全不同，不能替它判死。UDP/QUIC 类协议（`hysteria2` / `tuic` / `wireguard` / `masque`）本就不该由 TCP 探测决定生死，照旧"能解析即放行"。

---

### 🧯 2. 判据抽成纯函数 + 回归闸
- **`reaches_proxy_stage`**：握手候选的取舍现在是一条可读、可测的判据 —— TCP 通 → 进；私网 / 端口 0 → 淘汰；公网主机名但本地解不出地址 → 交给内核；解出地址却连不上 → 淘汰。
- **两条反向对照**：`tcp_failure_with_a_resolved_address_is_eliminated` 钉住"二阶段淘汰"本身；`private_hosts_never_reach_the_kernel` 是被仓库里那条跑了好几个月的老测试 `persists_tcp_prefilter_failure_as_unavailable` 教出来的 —— 第一版判据把"DNS 表里没这个键"和"私网主机"混成一类，让 `127.0.0.1:80` 复活去握手，那条老测试当场变红。
- 全量测试 **405 passed / 0 failed**，Windows / macOS / Linux / OpenWrt 四条 CI 全绿。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.11_*.ipk ./luci-app-vpn-parser_1.8.11_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

---

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.11_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.11_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.11_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.11_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.11_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.11-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.11-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.11-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.11-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.11-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.11-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.11-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.10...v1.8.11
