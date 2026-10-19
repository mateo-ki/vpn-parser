# 🚀 VPN Parser v1.8.6 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.6**！本次更新带来了**节点池极简双行工具栏整合**、**控制台仪表盘黄金比例对齐与防裁切圆角**、**出海健康与延迟频谱卡片等高重构**，以及 **Windows 原生网络崩溃自愈与 Job Object 僵尸进程防残留机制**！

---

## 🌟 新特性与核心更新

### 🎯 1. 节点池（Nodes）工具栏极简双行重构
- **搜索与核心操作合流同一行**：
  - 将「搜索框」、「订阅源下拉筛选」与「⚡ 批量测速 ▼」、「🧹 维护清理 ▼」、「📥 导入/导出 ▼」整合至第一行，纵向占用空间大幅压缩，操作链路一气呵成；
- **地区筛选与排序深度融合**：
  - 将「排序」下拉框移至地区胶囊（全部、香港、日本、美国、新加坡、台湾、韩国、其它）末尾，信息流逻辑归一，视觉层级更清爽；
- **界面极致降噪**：
  - 移除冗余的独立「刷新」按钮，移除订阅源下拉框旁的独立 `✕` 清除按钮，还原极简、开阔的高效浏览视野。

---

## 🎨 2. 控制台仪表盘黄金比例对齐与视觉优化
- **全网延迟频谱与协议分布严丝合缝等高（90px 外高 / 74px 内高）**：
  - 彻底解决原先因垂直居中对齐导致的协议卡片顶部大面积异常留白与左右卡片参差不齐的问题；
  - 顶部基准线水平对齐，协议标签自适应两行约束，紧凑平整；
- **双翼网络监视器高度严格对齐（138px）**：
  - 「实时下行带宽」仪表盘与「24h 出海健康」卡片实现全响应式等高；
- **滚动条视口外边距防裁切修复**：
  - 修复 egui Frame 内边距向外扩展导致的右侧圆弧与描边被视口裁切的问题，所有卡片在各种窗口尺寸下均呈现圆润平滑的四角微弧。

---

## 🛡️ 3. Windows 原生崩溃网络自愈与防僵尸内核强绑定
- **Windows SEH 崩溃原子回滚（In-Crash Guard）**：
  - 进程发生未捕获异常或闪退时，底层通过 Win32 原生注册表原子函数瞬间复位系统代理并广播刷新，杜绝崩溃断网；
- **启动自动探测与自愈（Startup Self-Healing）**：
  - 应用启动时自动探测残留代理端口存活性，若上一次运行非正常中断且端口已死，秒级静默清理复位；
- **Windows Job Object 强生命周期绑定**：
  - 为代理内核配置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`，保证无论主进程正常退出、异常闪退还是被任务管理器强杀，内核绝无可能在系统后台沦为僵尸进程。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.6_*.ipk ./luci-app-vpn-parser_1.8.6_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

---

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.6_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.6_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.6_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.6_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.6_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.6-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.6-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.6-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.6-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.6-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.6-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.6-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.5...v1.8.6
