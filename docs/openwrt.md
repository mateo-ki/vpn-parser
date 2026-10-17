# OpenWrt 部署与微服务联动指南

VPN Parser 在 OpenWrt 上以**纯无头（Headless）高性能守护进程**形式运行，作为路由器的**本地节点清洗中枢与上游订阅微服务**。

它在后台全自动定时抓取外部订阅、挖掘 GitHub Gist、并发测速、真握手探活、去重清洗，并将筛选出的最高质量活节点以标准的 Clash YAML 或 Base64 订阅协议吐给 **OpenClash**、**PassWall**、**SSR-Plus** 等插件，彻底解决路由器直接拉取外部订阅时卡顿、死节点多、测速拖垮路由 CPU 的痛点。

---

## 🌟 核心特性

- **极低资源占用**：剥离所有 GUI 渲染管线，纯静态 musl 编译，运行私有工作集内存仅约 **20MB ~ 30MB**，闲置 CPU 占用接近 0%；
- **OpenWrt 原生进程守护 (`procd`)**：集成 `/etc/init.d/vpn-parser` 服务，支持崩溃 5 秒自愈拉起（`respawn`）与优雅关停（`SIGTERM`）；
- **UCI 配置系统集成**：支持通过 `/etc/config/vpn-parser` 及 `uci` 命令行直接管理监听端口、存储路径与鉴权令牌；
- **系统现有内核自动复用**：自动探测路由器已安装的 `mihomo` / `sing-box` / `xray`（`/usr/bin` 等系统路径），无需重复下载内核，保护路由器 Flash 闪存空间；
- **只读文件系统自适应**：完美适配 SquashFS 只读根目录，临时文件与运行日志自动安全隔离至 `/tmp` 内存盘。

---

## 📦 支持架构与安装包

在项目的 [GitHub Releases](https://github.com/mateo-ki/vpn-parser/releases) 页面中提供预编译安装包：

| 架构名称 | 适用设备 | 安装包文件名 |
| :--- | :--- | :--- |
| **x86_64** | 通用 X86 软路由、工控机、PVE / ESXi 虚拟机等 | `vpn-parser_<版本>_x86_64.ipk` |
| **aarch64_generic** | 主流 ARM64 软硬路由（NanoPi R2S/R4S/R5S、友善、GL.iNet 等） | `vpn-parser_<版本>_aarch64_generic.ipk` |
| **静态压缩包** | 通用 Linux / OpenWrt（解压即用） | `vpn-parser_<版本>_linux_<架构>_musl.tar.gz` |

---

## 🚀 安装步骤

### 1. 安装 `.ipk` 软件包（推荐）

通过 SCP 或 SFTP 将下载好的 `.ipk` 上传到路由器的 `/tmp` 目录下，SSH 登录路由器执行：

```bash
cd /tmp
# 更新源并安装
opkg update
opkg install ./vpn-parser_*.ipk
```

安装完成后，服务会自动注册开机自启并立即启动。

### 2. 手动解压安装（无 opkg 环境）

若使用精简版固件，可直接使用纯静态归档：
```bash
# 解压到系统目录
tar -xzf vpn-parser_*_musl.tar.gz -C /
chmod +x /usr/bin/vpn-parser /etc/init.d/vpn-parser
/etc/init.d/vpn-parser enable
/etc/init.d/vpn-parser start
```

---

## ⚙️ 服务管理与 UCI 配置

### 服务控制命令

```bash
/etc/init.d/vpn-parser start    # 启动服务
/etc/init.d/vpn-parser stop     # 停止服务
/etc/init.d/vpn-parser restart  # 重启服务
/etc/init.d/vpn-parser reload   # 重载配置
/etc/init.d/vpn-parser status   # 查看运行状态
/etc/init.d/vpn-parser enable   # 启用开机自启
/etc/init.d/vpn-parser disable  # 禁用开机自启
```

### UCI 配置文件：`/etc/config/vpn-parser`

安装后默认配置文件内容如下：

```uci
config vpn-parser 'main'
	option enabled '1'
	option listen '0.0.0.0:8080'
	option db '/etc/vpn-parser/vpn-parser.db'
	option api_token ''
```

**修改配置示例**：
```bash
# 将监听端口修改为 8088
uci set vpn-parser.main.listen='0.0.0.0:8088'

# 设置访问密钥（可选，局域网共享时建议配置）
uci set vpn-parser.main.api_token='your-secret-token'

# 保存并生效
uci commit vpn-parser
/etc/init.d/vpn-parser restart
```

---

## 🔗 与路由器代理插件协同联动

VPN Parser 运行后，在本地监听 HTTP 端口（默认 `8080`），提供丰富的导出参数：

### 1. 配合 OpenClash 使用

1. 进入 OpenClash 页面 ➔ **「配置文件订阅」** ➔ 点击 **「添加」**；
2. 配置参数：
   - **配置文件名**：`VPN-Parser`
   - **订阅地址**：
     ```text
     http://127.0.0.1:8080/api/export/clash?quality_only=true&limit=60
     ```
   - 如设置了 `api_token`，在 URL 后追加 `&token=your-secret-token`；
3. 点击 **「保存配置」** ➔ **「更新配置」**；
4. OpenClash 将直接载入经过真握手验证、低延迟的优质纯净节点列表。

### 2. 配合 PassWall 使用

1. 进入 PassWall 页面 ➔ **「节点订阅」** ➔ 点击 **「添加」**；
2. 配置参数：
   - **订阅备注**：`VPN-Parser`
   - **订阅网址**：
     ```text
     http://127.0.0.1:8080/api/export/uri?quality_only=true&limit=60
     ```
3. 保存并点击 **「手动更新所有订阅」**；
4. PassWall 会自动解析所有 VLESS / VMess / Trojan / SS 节点。

### 3. 配合 ShadowSocksR Plus+ (SSR-Plus) 使用

- 进入 SSR-Plus ➔ **「服务端节点」** ➔ 订阅 URL 填入：
  ```text
  http://127.0.0.1:8080/api/export/uri?quality_only=true
  ```

---

## 📡 导出 API 参数速查

在订阅链接后添加查询参数，可精细化控制路由器拉取的节点规格：

| 参数名 | 默认值 | 示例 | 作用 |
| :--- | :--- | :--- | :--- |
| `quality_only` | `false` | `quality_only=true` | **核心推荐**：只导出握手成功、延迟正常的活节点 |
| `limit` | `50` | `limit=80` | 最大导出节点数（防止节点过多撑爆插件） |
| `max_latency` | - | `max_latency=300` | 仅保留实测延迟低于指定毫秒数的节点 |
| `min_download` | - | `min_download=5000` | 仅保留下载吞吐测速高于指定 kbps 的节点 |
| `country` | - | `country=HK,JP,US` | 指定国家/地区代码过滤（多国家逗号分隔） |
| `token` | - | `token=xxx` | API 认证令牌（若配置了 api_token 则必填） |

---

## 🛠️ 故障排查

1. **检查进程是否在运行**：
   ```bash
   pgrep -l vpn-parser
   ```
2. **检查端口监听状态**：
   ```bash
   netstat -tuln | grep 8080
   ```
3. **查看系统日志与启动输出**：
   ```bash
   logread | grep vpn-parser
   # 或查看临时目录日志
   cat /tmp/vpn-parser-startup.log
   ```
4. **测试本地回路 API 连通性**：
   ```bash
   curl -i http://127.0.0.1:8080/api/stats
   ```
