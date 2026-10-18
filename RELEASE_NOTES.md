# 🚀 VPN Parser v1.8.3 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.3**！本次更新重点优化了**节点选择与活动出口绑定的交互体验**，全面消除了点击误触，并重构了纯几何矢量日夜图标与控制台卡片交互体系！

---

## 🌟 新特性与核心更新

### 🎯 1. 节点池与控制台「可用节点」交互深度重构
- **单击仅选中，彻底杜绝误触**：
  - 在「节点池」及「控制台 ➔ 可用节点」卡片列表中，**左键单击卡片仅做选中高亮显示**，不再强行启动代理或更改网络，便于随心浏览与查阅节点参数；
- **右键菜单与双击直觉设为活动节点**：
  - 右键菜单首项新增 **`🚀 设为活动节点`**（若位于特定分流策略组则为 **`🚀 设为「策略组」活动出口`**）；
  - 保留桌面端经典操作：**双击节点卡片**同样可直达设为活动出口；
  - 悬停浮层提示同步对齐：`{IP}:{Port}\n左键点击选中，右键设为活动节点`。

---

### 🎨 2. 菜单栏底部日夜主题切换升级为纯几何原生矢量图标
- **彻底根治 Emoji 变体字符豆腐块（`☀️ □`）问题**：
  - 弃用容易引发操作系统字体字形缺失的 Unicode 变体选择符（`VS16 / U+FE0F`），全面升级为项目专属原生矢量绘制：
    - **`Icon::Sun`（日间）**：中心太阳圆盘与 8 根放射光芒几何线段；
    - **`Icon::Moon`（夜间）**：双弧度优雅切片弯月轮廓；
  - 完美适配任意 DPI 显示缩放，悬停自适应主题强调色高亮。

---

### 🔍 3. 节点池「订阅源筛选」与下拉即时模糊搜索
- **工具栏下拉选择**：在节点池顶部工具栏新增订阅源下拉过滤组件；
- **内嵌即时模糊搜索**：支持在展开后快速键入订阅源名称、备注或 URL 过滤目标源，修复了点击输入框导致弹层误关闭的问题（启用 `CloseOnClickOutside`）；
- **一键重置快捷按钮**：激活状态下提供一键 `✕` 清除按钮；
- **定向查询加速**：切换订阅源后定向查询该源所属的独立分片数据库，分页重置至第 1 页。

---

### ⚡ 4. 批量测速新增「测速当前列表」
- **矩阵更完备**：
  - **`⚡ 测速当前页`**：快速测试当前页（50个）节点；
  - **`⚡ 测速当前列表`**：自动匹配当前筛选条件（协议、源、关键词、地区等）跨越全部页数的**所有节点**并发测速；
  - **`⚡ 测速全库全部节点`**：全量全源并发测速。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.3_*.ipk ./luci-app-vpn-parser_1.8.3_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.3_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.3_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.3_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.3_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.3_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.3-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.3-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.3-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.3-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.3-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.3-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.3-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.2...v1.8.3
