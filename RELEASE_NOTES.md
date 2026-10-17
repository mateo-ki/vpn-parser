# 🚀 VPN Parser v1.8.1 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.8.1**！本次更新正式推出了 OpenWrt 官方原生 LuCI Web 图形管理插件 **`luci-app-vpn-parser`**，提供全架构通用的现代客户端图形交互界面，彻底打通路由器 Web 管理面板与后台微服务之间的所有壁垒！

---

## 🌟 新特性与核心更新

### 🖥️ 1. 原生 OpenWrt LuCI Web 图形管理插件 (`luci-app-vpn-parser`)
- **全架构通用安装包 (`Architecture: all`)**：
  - 由纯静态 JS 视图组件、UCI 接口与 ubus 桥接器构成，一份 `luci-app-vpn-parser_1.8.1_all.ipk` 通用兼容 x86_64、ARM64（NanoPi/友善/GL.iNet）及 MIPS 全架构设备；
- **现代客户端渲染架构 (`luci.js`)**：
  - 基于 OpenWrt 21.02+ 现代标准客户端渲染引擎，彻底移除旧版 Lua CBI 的沉重负担与性能损耗；
- **免跨域与零泄露设计 (rpcd ubus 桥接守护)**：
  - 核心通信经由路由器本地 `uhttpd` 的 `/cgi-bin/luci/rpc/ubus` 转发，完全免疫浏览器 CORS 跨域拦截与 HTTPS 混合内容限制；
  - API Token 安全地保留在路由器本地读取与注入，无需在浏览器端明文暴露；
- **四大核心控制面板**（位于路由器后台 **「服务 (Services)」➔「VPN Parser」**）：
  1. **服务概览 (`Overview`)**：实时 PID 与端口守护徽章、订阅与节点质量看板、各协议分布药丸、一键全量测速/清理失效节点快捷按钮、预设下游插件一键复制订阅卡片（OpenClash YAML / PassWall Base64 URI / JSON）；
  2. **节点池管理 (`Nodes`)**：多维状态过滤（全部/仅看可用/超时失效/未测速）、协议过滤、关键字即时检索，单节点独立真握手测速、分享链接复制与删除；
  3. **订阅源管理 (`Sources`)**：订阅源增删改查交互模态框（支持动态日期模板占位符）、单源即时抓取、全网 GitHub Gist 公开节点自动搜集与入库；
  4. **参数配置 (`Settings`)**：UCI 服务守护设置（开机自启、监听地址、数据库路径、Token）与测速引擎高级调优（并发数、纯 TCP Ping 节能探活模式、同出口 IP 去重等）一键保存并应用；
- **自动化构建与发布**：
  - CI/CD 流水线自动打包 `luci-app-vpn-parser_1.8.1_all.ipk` 并自动发布至 GitHub Releases 资产。

---

## 📦 各平台下载与安装指引

### 1. OpenWrt 路由器（极简安装）

在 OpenWrt 终端中运行以下命令即可同时安装微服务守护进程与 LuCI 控制面板：

```bash
cd /tmp
opkg update
# 一键安装微服务守护进程与 LuCI 控制面板
opkg install ./vpn-parser_1.8.1_*.ipk ./luci-app-vpn-parser_1.8.1_all.ipk
```
安装后刷新路由器 Web 管理后台，即可在 **「服务」➔「VPN Parser」** 中开始使用。

### 2. 资产列表

| 平台 / 架构 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **OpenWrt LuCI 控制面板** | `luci-app-vpn-parser_1.8.1_all.ipk` | 推荐，全架构通用 LuCI Web 图形界面 |
| **OpenWrt 微服务 (x86_64)** | `vpn-parser_1.8.1_x86_64.ipk` | 推荐，X86 软路由、工控机、PVE/ESXi 一键安装 |
| **OpenWrt 微服务 (aarch64)** | `vpn-parser_1.8.1_aarch64_generic.ipk` | 推荐，ARM64 路由（NanoPi R2S/R4S/R5S、友善等）一键安装 |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.1_linux_x86_64_musl.tar.gz` | x86_64 纯静态 Musl 二进制包（含 init/config 脚本） |
| **OpenWrt/Linux 静态归档** | `vpn-parser_1.8.1_linux_aarch64_generic_musl.tar.gz` | ARM64 纯静态 Musl 二进制包 |
| **Windows 64位** | `vpn-parser-1.8.1-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（桌面完整版） |
| **macOS (Universal DMG)** | `vpn-parser-1.8.1-darwin-universal.dmg` | 推荐，兼容 Apple Silicon 与 Intel 架构 |
| **Linux 桌面 (通用)** | `vpn-parser-1.8.1-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.8.1-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.8.1-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.8.1-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.8.1-1_amd64.deb` |
| **校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 校验和 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.8.0...v1.8.1
