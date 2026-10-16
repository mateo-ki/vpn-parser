# VPN Parser

Rust + egui 写的桌面程序：采集订阅、解析节点、跑代理、把代理质量实测出来。
Windows 是主目标（安装包 + 系统代理 + TUN）；Linux 已能编译、运行并自带 deb/rpm 打包，
但目前只提供本地混合端口（把应用指向 `127.0.0.1` 的监听端口），TUN 还没接入。

## 功能

**采集**
- 订阅 URL（支持 `YYYY-MM-DD` 日期占位符）
- Gist 搜索页发现：把 gist raw 直接登记成订阅源
- Telegram 频道：读 `t.me/s/<频道>` 网页预览，裸节点链接和订阅链接都取，节点只增不删

**测速与健康**
- 先 TCP 预筛，再经内核访问 Google `generate_204` 计延迟，可选下载吞吐探测
- 出口健康条按最近 24 小时实测画，不看进程状态挂绿勾
- 出口切换记录落库留 90 天，按本地日分组，含"该切但没找到替补"

**选路**
- 内核层：`PROXY(select) → 自动选择(url-test) → 漏网之鱼`，换节点不重读配置
- 应用层：故障转移调度器（EWMA + 断路器 + 滞后 + 最小切换间隔）
- 替补池空了可自动补一轮低并发全量测速（默认关）

**分流规则**
- GEOSITE / GEOIP / 域名 / 端口 / 进程名等匹配器，规则可增删改
- 每条规则实际命中多少连接单独一页显示
- 导入 / 导出：吃 mihomo 规则文本、JSON 数组、Clash Verge 扩展脚本，解析后先给预览再落库

**代理与内核**
- 系统代理、TUN（提权交接后只留一个实例）、终端 `HTTP_PROXY` 三条都管
- 内核缺失时开下载窗口：ghfast / gh-proxy / 官方直连 / 走代理 / 本地导入，一次只走一条
- 按 exe 路径加防火墙允许规则；单实例互斥；托盘

**软件自身**
- 关于页在线更新，稳定与预发两个通道（默认只跟稳定版）
- 界面全中文、5 套浅色主题、Ctrl+K 命令面板、85%~140% 整页缩放
- 本地 HTTP API（默认 `127.0.0.1:8080`，强制 bearer 鉴权）
- 导出 mihomo / sing-box / xray 配置

## 协议

解析：`ss` `vmess` `vless` `trojan` `hysteria2/hy2` `socks5` `http(s)`，以及 Clash/Clash Meta YAML、Base64 与混合文本里的分享链接。
连接（取决于内核）：以上再加 `tuic` `wireguard`。`anytls` `ssr` `mieru` `openvpn` `sstp` 存得下但连不上，界面上会标出来而不是静默丢弃。

## 运行与打包

```bash
cargo run --release                 # 直接跑
cargo build --release
"<Inno Setup>\ISCC.exe" installer/vpn-parser.iss   # 安装包，产物在 dist/
```

数据都在程序目录的 `data/` 下：主库 `vpn-parser.db`（订阅源、设置、遥测），节点按源分片存 `data/subscriptions/*.db`。

## 上手

1. 订阅源页加一条源（或跑一次 Gist / Telegram 发现），点拉取
2. 节点池点一个节点作为出口 → 启动代理（没装内核会自动开下载窗口）
3. 浏览器打不开境外时，先看表头那行实测健康，而不是看它有没有绿

## 已知边界

- 内核装在用户可写目录（`data/kernels`），TUN 提权时会以管理员执行其中的 exe
- url-test 只看"到测试 URL 的 HTTP 延迟"，不看带宽也不看落地 IP 信誉
- 免费共享订阅失效极快，测通数量长期偏低是常态

## 社区

本开源项目已链接并认可 [LINUX DO 社区](https://linux.do)。
