# 🚀 VPN Parser v1.8.4 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.4**！本次更新带来了重量级的**控制台安全隐私四重防御**、**活动节点列表（一节点一端口独立转发）**，并实现了**极速内存缓存**与**可用节点分页及 5 列自适应黄金比例卡片排版**！

---

## 🌟 新特性与核心更新

### 🛡️ 1. 控制台新增「安全隐私」Tab 与四重防御体系
- **全局 uTLS 客户端指纹自动伪装 (Client Fingerprint)**：
  - 自动为出站节点注入真实浏览器 TLS 握手特征（Chrome / Safari / Firefox / Edge / Randomized 随机混淆）；
  - 抹平代理连接特征，有效抵御针对梯子流量的 DPI 深度包检测与启发式特征识别；
- **WebRTC 真实 IP 泄漏防御 (Anti-Leak)**：
  - 内核层全局拦截 STUN (3478) 与 TURN (5349) 探测端口及特征域名；
  - 彻底斩断网页恶意 JavaScript 探针刺探本地真实公网 IP 的链路；
- **隐私与行为追踪广告规则集过滤 (Tracker & Telemetry Blocking)**：
  - 内核级阻断常见数据统计埋点、用户行为追踪与广告遥测域名；
- **UDP 端口跳跃与防 QoS 丢包混淆**：
  - 针对 Hysteria2 / TUIC 等 UDP 协议，支持设定自定义端口跳跃范围（如 `20000-40000`），规避运营商单端口限速与 QoS 丢包。

---

### 🔌 2. 控制台新增「活动节点列表」（一节点一端口独立转发）
- **多入站端口独立转发 (Port Mapping)**：
  - 可为任意特定节点分配专属的本地监听端口（例如 `127.0.0.1:10810`）；
  - 外部爬虫程序、浏览器扩展插件或开发脚本连接该端口时，流量将**独占经由绑定的节点转发**，完全绕过全局代理路由；
- **优雅启动与容灾保障**：
  - 深度优化多监听端口注入机制，保证内核热重启时不丢失映射关系。

---

### ⚡ 3. 内存级高速节点缓存 (NodeSummaryCache) 与零拷贝过滤
- **5000x 检索性能提升**：
  - 在 `Database` 引入轻量级内存摘要缓存 `NodeSummaryCache`，首次读取自动预热；
  - 增删查改、批量测速、导入更新全生命周期自动失效保障一致性；
  - 节点池过滤、排序与分页开销从过去的 >1000ms 磁盘分片全表扫描直接骤降至 **106 微秒 (0.1ms)**。

---

### 📑 4. 控制台「可用节点」新增分页与自适应 5 列黄金比例卡片
- **可用节点分页支持**：
  - 控制台可用节点卡片列表支持分页浏览，具备顶部与底部双向快速翻页按钮；
  - 支持快捷切换每页容量（`20 / 25 / 30 / 50 / 100 条/页`），切换筛选/搜索条件即时重置回第 1 页；
- **自适应 5 列黄金排版**：
  - 卡片自适应基准宽度调整为 `270px`，在常用分辨率及窗口下一行稳定呈现约 **5 张卡片**，兼顾充裕的节点名展示与优雅的紧凑感。

---

### 🎨 5. 月相夜间图标平滑几何重构与性能深度清理
- **Moon 夜间图标无畸变绘制**：
  - 采用精确几何交点与双弧度凸多边形网格切片算法重构，彻底根除高分屏缩放下的折线尖刺与锯齿；
- **60 FPS 逐帧深拷贝消除**：
  - 消除界面渲染主循环中的所有每帧深拷贝，清理废弃死代码，实现全工程 **0 警告 Clean Build**。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.4_*.ipk ./luci-app-vpn-parser_1.8.4_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.4_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.4_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.4_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.4_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.4_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.4-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.4-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.4-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.4-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.4-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.4-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.4-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.3...v1.8.4
