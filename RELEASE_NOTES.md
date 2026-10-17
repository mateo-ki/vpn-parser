# 🚀 VPN Parser v1.7.3 发布说明 / Release Notes

🎉 欢迎使用 **VPN Parser v1.7.3**！此版本对控制台进行了全新图形化与交互重构，重塑了仪表盘视觉中枢，下沉调度决策中心并引入全自动路由起振，彻底消除界面抖动，带来极致丝滑的桌面网络管理体验；同时全面支持 macOS 原生字体渲染与 DMG/App 打包分发。

---

## 🌟 新特性与核心更新

### 📊 1. 控制台视觉中枢全新重构 (Console Dashboard Overhaul)
- **图表默认直达 (Default Charts View)**：进入「控制台」默认聚焦「图表」仪表盘，实时呈现网络测速仪表盘、平滑吞吐波形图与出海健康带；
- **分流策略与调度决策中心下沉**：将策略组与「延迟选优」「故障转移」「负载均衡」「手动指定」按钮平滑迁入图表底部，最大化释放「可用节点」视图的垂直纵深；
- **一键调度智能起振代理 (Auto-Start Proxy on Strategy)**：未运行代理时，点击「延迟选优」「故障转移」或「负载均衡」立即自动启动代理并应用调度，一步到位；
- **智能点选无感切手动 (Auto-Switch to Manual on Node Click)**：在自动选优/故障转移/负载均衡模式下，直接点击任意可用节点卡片，自动将策略切换为「手动指定」并瞬间联通出口；
- **示波器平直待机基线 (Oscilloscope Standby Flatline)**：代理待机或无流量时，监视器绘制平直示波基线 (0.0 KB/s) 配合辉光游标与峰值量程标尺，替代大字提示，告别视觉割裂感；
- **主控卡片固定高度杜绝抖动 (Zero-CLS Hero Island)**：移除顶部冗余按钮，第二行始终保持 34px 固定高度微晶胶囊，未选节点时展示占位监控指标，彻底杜绝选点后的界面跳动；
- **全网延迟频谱与协议环图严格等高**：左右卡片高度严格锁定为 104px 对齐；右翼协议药丸智能紧凑排版（最多两行，多协议聚合成「其它 N (占比%)」并支持悬停明细查看），彻底解决小众协议换行撑爆高度的问题。

---

### 🌐 2. 活跃连接多维视图拆分 (Connections View Multi-Tabs)
- **连接列表 / 连接进程 / DNS 记录三重视图**：支持实时连接会话监控、按应用进程聚合统计上行与下行流量，以及核心 DNS 缓存解析记录。

---

### 🍎 3. macOS 深度适配与跨平台优化
- **macOS 中文字体完美渲染**：新增 PingFang SC、Hiragino Sans GB、STHeiti 等系统字库探测与多级字体回退链，彻底根治 macOS 下中文字体缺失显示为方块（豆腐块）的问题；
- **macOS 原生红黄绿交通灯与调色板**：适配 macOS 原生风格的主题调色板与窗口控制细节；
- **长页面自适应滚动**：DNS/TUN 设置页与内核管理页全卡片引入自适应上下滚动，避免下载进度条或复杂规则挤压隐藏底部内容。

### 🐧 4. Linux 平台安全与系统级体验专项调优 (Fedora / GNOME / niri / DMS)
- **进程清理安全隔离（杜绝误杀第三方客户端）**：内核清理增加 `/proc/<pid>/exe` 路径与 `cmdline` 特征强校验，严格仅清理本程序受管目录下的内核，绝不误杀 Clash Verge、v2rayA、nekoray 等第三方客户端内核；
- **TUN 授权引导与 Wayland 崩溃修复**：严禁使用 `sudo` 启动 GUI，避免 Wayland 拒绝授权及数据目录错乱；改为向用户提供内核二进制文件的精确 `sudo setcap cap_net_admin,cap_net_raw+ep <kernel_path>` 授权方案；
- **GSettings Linux 桌面系统代理原生支持**：为 Linux 平台实现基于 GSettings（`org.gnome.system.proxy`）的系统代理自动开启与无缝还原，让 Fedora / Ubuntu / Debian / niri / DMS 下的浏览器与桌面应用均能一键自动走代理；
- **Unix Domain Socket 单实例互斥与平滑唤醒**：废弃粗暴终止旧实例的做法，改用 Unix Domain Socket 发送唤醒指令激活已有前台窗口并优雅退出，保护正在运行的代理与数据库写入；
- **平铺窗口管理器（niri / Hyprland）自适应排版**：窗口最小保护尺寸优化为 `820 × 520`，控制台策略组与调度按钮支持响应式折行，避免半屏平铺时元素溢出。

---

## 📦 各平台下载指引

| 平台 | 文件名 | 说明 |
| :--- | :--- | :--- |
| **Windows 64位** | `vpn-parser-1.7.3-windows-x64-setup.exe` | 推荐，双击运行向导自动安装升级（含 VirtualBox 兼容自适应） |
| **macOS (DMG)** | `vpn-parser-1.7.3-macos-x64.dmg` | 推荐，拖拽至 Applications 即可运行 |
| **Linux (通用)** | `vpn-parser-1.7.3-x86_64.AppImage` | 推荐，开箱即用，赋予可执行权限后双击运行 |
| **Linux (Fedora/RHEL)** | `vpn-parser-1.7.3-1.x86_64.rpm` | `sudo dnf install ./vpn-parser-1.7.3-1.x86_64.rpm` |
| **Linux (Debian/Ubuntu)** | `vpn-parser_1.7.3-1_amd64.deb` | `sudo dpkg -i vpn-parser_1.7.3-1_amd64.deb` |
| **完整校验文件** | `sha256.txt` | 包含所有发布资产的官方 SHA-256 哈希值 |

---

**Full Changelog**: https://github.com/mateo-ki/vpn-parser/compare/v1.7.2...v1.7.3
