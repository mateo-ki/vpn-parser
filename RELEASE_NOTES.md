# 🚀 VPN Parser v1.8.5 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.5**！本次更新带来了**应用图标自由切换（经典黑曜石 & 流光香槟金）**、**控制台图表高度黄金比例优化**、**莫兰迪低饱和柔和护眼主题**、**纯字体排版双色节点国旗微徽章**，以及**全景温润木炭黑图标重构**！

---

## 🌟 新特性与核心更新

### 🎨 1. 应用图标自由切换与流光香槟金 App Logo 设计
- **系统设置新增「应用图标风格」选择器**：
  - 位于「系统设置 ➔ 外观设置」，提供 **「经典黑曜石 (原版)」** 与 **「流光香槟金 (推荐)」** 两种官方视觉风格；
  - 单选切换后即时触发内存纹理重载，左侧导航栏与关于界面的主图标**无需重启即刻平滑热更新**；
- **流光香槟金 App Logo 重塑**：
  - 采用流光香槟金微渐变金属倒角字型，搭配金质边缘内发光与柔和深黑曜石微弧背景，彻底告别原版生硬纯黑对比，整体质感典雅高级。

---

### 📊 2. 控制台图表高度优化与紧凑黄金比例排版
- **卡片高度自适应压缩 50px**：
  - 「全网延迟阶梯频谱分布」与「节点协议组成分布」容器高度从 164px 优化至 **114px**；
  - 协议环形图直径等比微调至 80px，消除上下无效留白，两张卡片保持完美等高，首屏视野更宽广。

---

### 🌿 3. 界面配色主题重构与莫兰迪低饱和护眼主题
- **新增 3 款精心调配的视觉主题**：
  - **莫兰迪低饱和 (柔和护眼 · 语义保留)** (`#52796F`)：低对比柔和灰绿，长时间注视不疲劳；
  - **Surge 拓扑绿** (`#34C759`)：生动明快的经典网络拓扑活力绿；
  - **Apple Store 晶紫** (`#A855F7`)：现代科技感十足的晶莹冷紫；
- **全景图表与健康状态条联动**：
  - 控制台延迟阶梯柱、节点健康条色彩体系全面呼应当前激活主题，整体视觉高度统一。

---

### 🏷️ 4. 节点卡片国家代码国旗色方案 B (Pure Typography)
- **极简双色无框字母**：
  - 彻底根除 Windows 原生 Segoe UI Emoji 黑白线框降级的视觉缺陷；
  - 采用国旗代表色与纯正大写字型排印，在各系统上均呈现像素级细腻的微徽章质感。

---

### 🖋️ 5. 全局柔和微炭黑图标（Soft Charcoal Black）
- **告别纯黑高硬度刺眼对比**：
  - 图标主暗部全面升级为 Apple 级温润木炭黑 (`#4C4C52`) 与柔和次级黑 (`#707076`)；
  - 笔触粗细自适应由 1.8px~2.2px 精准收细至 1.15px~1.40px，并在 Dashboard 与 Lightning 等图标上引入镂空双层透气结构，视效更加轻盈通透。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.5_*.ipk ./luci-app-vpn-parser_1.8.5_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

---

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.5_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.5_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.5_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.5_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.5_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.5-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.5-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.5-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.5-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.5-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.5-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.5-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.4...v1.8.5
