# 🚀 VPN Parser v1.8.2 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.2**！本次更新深度优化了**节点池检索、多源过滤与批量测速体系**，同时为桌面端带来了更符合直觉的快捷操作与视觉交互体验！

---

## 🌟 新特性与核心更新

### 🔍 1. 节点池增加「订阅源筛选」与下拉即时模糊搜索
- **工具栏无缝集成**：在节点池顶部工具栏新增「订阅源」下拉选择器；
- **下拉内嵌即时模糊搜索**：
  - 点击展开下拉列表后，顶部常驻搜索输入框，可针对订阅源名称、备注及订阅 URL 进行实时模糊检索，下方列表支持平滑滚动；
  - 彻底解决点击输入框导致弹层意外收起的问题（采用 `CloseOnClickOutside` 行为），输入关键词后从容筛选并点击目标源；
  - 选项显示「订阅源名称 (节点数)」，在筛选激活状态下提供一键 `✕` 快速重置按钮；
- **分库精准定向**：选中订阅源后，节点池立即定向查询该源所属的独立分片数据库，毫秒级响应并自动重置到第 1 页。

---

### ⚡ 2. 批量测速新增「测速当前列表」（并保留测速当前页）
- **功能矩阵完善**：
  - **`⚡ 测速当前页`**：**完整保留**，仅针对当前可视分页的 50 个节点进行快速真实握手验证；
  - **`⚡ 测速当前列表`**：**全新增加**，对当前已激活的所有筛选条件（订阅源、协议类型、关键字、连通状态、国家地区）在数据库中匹配到的**全量节点（跨越分页）**发起并发真握手测速，并在状态栏实时推送测速进度与最终存活统计；
  - **`⚡ 测速全库全部节点`**：全量全源并发测速管线保持不变。

---

### 🌗 3. 菜单栏最底部新增「日间 / 夜间主题切换」
- **直观布局**：在左侧主菜单栏最底部放置主题切换图标按钮，与主导航图标垂直对齐，上方附带精致细分界线；
- **动态图标与悬停气泡**：
  - 当前处于深色模式时显示 `☀️`（提示“切换至日间亮色模式”）；
  - 当前处于浅色模式时显示 `🌙`（提示“切换至夜间深色模式”）；
- **即时响应与持久化**：点击即刻在下一帧无感重载整套主题色彩，并自动持久化写入数据库设置。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.2_*.ipk ./luci-app-vpn-parser_1.8.2_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.2_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.2_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.2_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.2_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.2_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.2-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.2-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.2-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.2-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.2-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.2-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.2-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.1...v1.8.2
