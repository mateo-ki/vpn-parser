# 代理运行 MVP 设计

## 设计原则

1. 首期以 Mihomo 为唯一常驻运行核心。
2. Xray 和 Sing-box 先进入内核管理模型，不参与首期运行路径。
3. 代理协议、TUN、规则、Geo、HTTPS DNS 尽量交给 Mihomo 原生能力，项目负责编排配置和生命周期。
4. 系统代理修改必须可恢复，启动失败不能留下脏系统状态。
5. 配置生成、进程管理、GUI 状态更新分层实现，避免继续扩大 `gui/mod.rs` 的职责。

## 模块划分

### `kernel`

负责内核资产管理。

主要职责：

1. 检测 Mihomo、Xray、Sing-box 路径。
2. 执行 `--version` 或等价命令读取版本。
3. 保存用户自定义内核路径。
4. 为后续更新器提供内核元数据。

核心类型建议：

```rust
enum KernelKind {
    Mihomo,
    Xray,
    SingBox,
}

struct KernelInfo {
    kind: KernelKind,
    path: Option<PathBuf>,
    version: Option<String>,
    available: bool,
    message: String,
}
```

### `proxy`

负责常驻代理运行。

主要职责：

1. 从数据库节点生成 Mihomo 配置。
2. 分配或读取本地 HTTP/SOCKS/Mixed/API 端口。
3. 启动、停止、重启 Mihomo 进程。
4. 读取启动日志和运行错误。
5. 对 GUI/API 暴露运行状态。

核心类型建议：

```rust
enum ProxyMode {
    System,
    Tun,
}

enum ProxyState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

struct ProxyRuntimeStatus {
    state: ProxyState,
    mode: ProxyMode,
    node_id: Option<String>,
    mixed_port: u16,
    controller_port: u16,
    message: String,
}
```

### `system_proxy`

负责 Windows 系统代理开关。

主要职责：

1. 启动系统代理时记录原始代理设置。
2. 将系统代理指向 `127.0.0.1:<mixed_port>`。
3. 停止代理或进程失败时恢复原始设置。
4. 避免重复启动导致状态覆盖。

首期只实现 Windows，其他平台返回明确错误。

### `rules`

负责分流规则模型和配置生成。

首期只保留默认规则结构，不做完整编辑器。

后续支持：

1. `DOMAIN`、`DOMAIN-SUFFIX`、`GEOSITE`。
2. `GEOIP`、`IP-CIDR`。
3. `DST-PORT`、`PROCESS-NAME`。
4. `NETWORK` 区分 TCP/UDP，并为 QUIC 提供 UDP/443 规则模板。
5. 动作：`DIRECT`、`PROXY`、`REJECT`。

### `dns`

负责 DNS 配置生成。

后续支持：

1. 普通 UDP DNS 与 HTTPS DNS；默认使用 `8.8.8.8`、`114.114.114.114`。
2. 国内外 DNS 分组。
3. Fake-IP 或 redir-host 模式选择。
4. 配合 GeoSite/GeoIP 做 DNS 分流。

### `updater`

负责内核更新。

实现约束：

1. 只从 Mihomo、Xray、Sing-box 官方 GitHub Release 获取 Windows x64 ZIP。
2. 使用 GitHub 资产元数据中的 SHA-256 摘要校验下载内容。
3. 内核统一安装到 `data/kernels/<kind>`，不扫描或复用 v2rayN。
4. 下载采用分块读取，GUI 展示字节进度、速度、校验、解压和安装阶段。
5. 用户可取消下载；取消发生在替换前，不修改现有内核。
6. Mihomo 运行时拒绝更新自身；Xray、Sing-box 在代理运行时通过本地 Mixed 端口下载。
7. Release 元数据写入本地缓存，GitHub API 限流或临时断网时回退缓存。
8. 替换前备份旧版本，新内核验证失败或替换失败时自动回滚。

## Mihomo 配置生成

首期生成最小可运行配置：

```yaml
mixed-port: 7890
external-controller: 127.0.0.1:9090
allow-lan: false
mode: rule
log-level: info
proxies:
  - <selected node converted to clash proxy>
proxy-groups:
  - name: PROXY
    type: select
    proxies:
      - <selected proxy name>
rules:
  - MATCH,PROXY
```

配置转换优先复用现有 `nodes_to_clash_proxies`。

## GUI 设计

首期新增一个“代理”页或在侧栏增加代理入口。

界面内容：

1. 当前代理状态。
2. 当前使用节点。
3. Mihomo 路径和版本。
4. 系统代理模式开关。
5. 启动、停止、重启按钮。
6. 最近日志和错误提示。

## API 设计

首期新增 API：

1. `GET /api/kernels`：查看内核状态。
2. `POST /api/proxy/start`：启动代理。
3. `POST /api/proxy/stop`：停止代理。
4. `POST /api/proxy/restart`：重启代理。
5. `GET /api/proxy/status`：查看运行状态。

## 数据持久化

建议扩展设置表或新增简单配置表，保存：

1. Mihomo 路径。
2. Xray 路径。
3. Sing-box 路径。
4. 默认 mixed port。
5. 默认 controller port。
6. 默认代理模式。
7. 上次使用节点 ID。

## 错误处理

必须明确处理：

1. Mihomo 不存在。
2. Mihomo 版本读取失败。
3. 节点无法转换为 Clash/Mihomo 配置。
4. 端口占用。
5. 进程启动失败。
6. 系统代理设置失败。
7. 进程异常退出。

## 验证策略

1. 单元测试覆盖配置生成。
2. 单元测试覆盖内核路径检测逻辑中的纯函数。
3. 集成式测试尽量避免真实修改系统代理。
4. 系统代理控制封装为 trait，测试使用 fake 实现。
5. 手动验证 Mihomo 启停和系统代理恢复。

## 第二阶段设计补充

### 配置边界

`src/proxy/config.rs` 定义 TUN、HTTPS DNS 和流量规则模型。规则使用有序列表保存，生成 Mihomo 配置时逐条校验并保持顺序，最后追加 `MATCH,PROXY`。

### 双模式启动

系统代理模式继续保存和恢复 WinINet 设置。TUN 模式生成 Mihomo `tun` 配置且不修改系统代理；Windows 非管理员进程通过 `ShellExecuteW` 的 `runas` 动作启动提权实例，内部参数携带完整代理配置与节点 ID，提权实例恢复配置后自动启动。

### 持久化与 API

SQLite `proxy_settings` 单行表保存完整 JSON 配置。应用启动优先读取该配置，GUI 与 `PUT /api/proxy/config` 更新时同时修改 `ProxyManager` 和数据库，数据库写入失败时回滚内存配置。

## 节点订阅 Tab 设计

### 数据查询

复用订阅统计模型，包含 `source_id`、订阅名称、节点总数和真实代理验证可用数。数据库使用 `sources LEFT JOIN nodes` 聚合，并聚合 `source_id IS NULL` 的未分组节点。统计查询沿用协议、关键词和延迟状态筛选条件，确保 Tab 计数与当前视图一致。

订阅测速不新增测速实现。数据库按 `source_id` 和当前筛选条件查询节点 ID，GUI 将这些 ID 传给现有 `SpeedTester::test_many`。未分组使用 `source_id IS NULL` 查询。

### GUI 交互

节点页在筛选工具栏前渲染横向可滚动订阅 Tab。Tab 显示订阅名称及“可用 N / 共 M”，点击后设置 `selected_source_id`、重置到第一页，并在数据库查询中传入该订阅 ID。工具栏提供“测速当前订阅”，测速期间沿用全局忙碌状态。节点表格恢复为单一表格，只包含当前订阅节点。

### 分页约束

分页查询必须带当前 `source_id`，因此页数和总数只属于当前订阅。测速当前订阅覆盖该订阅中全部符合筛选条件的节点，不受分页限制。

### 测速状态修复

第一阶段结束后，将未进入真实代理验证的失败项以 `latency_ok=false`、`latency_test_kind='tcp_prefilter'` 批量写入数据库。成功进入第二阶段的节点只在真实验证完成后覆盖为 `real_proxy` 结果。这样 TCP 失败不会继续显示“未测速”，同时“可用”统计仍只认 `real_proxy`。

### 代理页两级选择

新增数据库查询返回仅含可用节点的订阅摘要，以及按 `source_id` 查询的完整可用节点列表。代理页维护独立的 `proxy_source_id`，不再依赖节点页当前分页。第一行渲染可用订阅 Tab，第二部分渲染该订阅节点卡片；无可用订阅时显示空状态。

## 2026-08-27 性能与多内核设计

### 拉取快速路径

`AppSettings` 保存拉取并发和单源总超时。`sources` 保存最近一次最终 URL、ETag、Last-Modified 与 SHA-256；URL 相同才发送条件请求。HTTP 304 或正文哈希相同均直接完成任务，不进入解析和分库替换。解析使用有界信号量限制阻塞任务数量。

### 代理会话与降级状态

系统代理快照的生命周期独立于单个内核进程。首次启用时捕获，节点切换和连接失败不恢复，显式停止与退出才恢复。`ProxyState::Degraded` 表示系统代理会话仍保留，但外部验证失败或当前没有可用本地监听；状态同时携带 `local_proxy_ready`、`system_proxy_enabled` 与 `last_error`。

统一 `connect` 操作根据当前状态执行首次启动或切换。外部验证失败发生在本地端口就绪之后时保留子进程和工作目录；配置、端口和进程启动等硬失败仍清理候选进程。节点列表直接调用该操作。

GUI 在执行首次启动、重启或节点切换前统一检查目标模式。目标为 TUN 且当前进程未提权时，序列化最新代理配置和节点 ID，通过 `runas` 启动提权实例，再关闭原窗口；不能因为操作类型是重启而绕过提权。

### 内核适配

运行层通过统一分派封装协议支持、配置生成、序列化、命令参数、就绪端口与验证。新增 `KernelPreference`（Auto/Mihomo/SingBox/Xray）。Xray 使用 HTTP 入站承载系统代理，首期支持 SS、VMess、VLESS、Trojan、HTTP、SOCKS；TUN 下自动跳过，手动选择则返回错误。

### 测速批处理

Mihomo 每批最多 256 个节点且最多 3 批并行。Sing-box 与 Xray 每批最多 64 个节点，由一个进程创建多个本地入站，并通过入站标签绑定对应出站。TCP 预筛在单次任务内缓存 DNS 结果并并行尝试地址。最终结果按 `source_id` 分组，在对应节点数据库中各使用一个事务写入。

### 并发设置持久化

设置页分别编辑 `fetch_concurrency`（4–64）和 `speedtest_concurrency`（1–512）。保存操作通过 `Database::update_settings` 更新 SQLite `settings` 单行记录，保存完成后重新加载内存设置；应用启动、手动任务、API 和定时任务均从同一记录读取，避免界面值与实际执行值不一致。

### 分流策略与节点操作层级

`MihomoConfigOptions` 增加带 Serde 默认值的 `RoutingMode`，包含 `Rule`、`ChinaDirect`、`GfwList`、`Global`、`Direct`。自定义规则以代理兜底；中国大陆直连使用 GeoSite/GeoIP 和广告分类并以代理兜底；GFW 列表命中走代理并以直连兜底；全局与直连模式只改变最终出口。在线预设不展开写入数据库，切换策略也不删除用户规则。三个内核的配置适配器根据同一字段生成最终规则，Xray 同时补齐域名、后缀、GeoSite、GeoIP、CIDR、端口、网络和 QUIC 映射；不支持的进程名规则返回明确错误。

GUI 的分流页先显示三段策略选择，再按需显示规则编辑器。节点表格操作列采用一个“连接”主按钮和一个次级菜单；顶部将低频、破坏性的清理操作归入“清理”菜单，降低误触和横向拥挤。

### 订阅重试与任务收敛

单源拉取与批量拉取使用不同的 GUI 完成消息，单源完成消息始终携带 `source_id`，只解除对应行并刷新结果，不修改批量 `busy`。GUI 在提交时立即把源加入单源执行集合，并与 `TaskTracker` 的非终态联合判断按钮状态，因此批量中已失败的源可以立即重试，排队、执行中或刚提交的同一源不会重复拉取。执行中的按钮显示“拉取中...”或“重试中...”。

Collector 对临时网络错误和可重试 HTTP 状态执行最多三次请求，退避期间仍属于同一个源任务。用户设置控制批量总并发，按解析后的 URL 主机再使用共享信号量限制为 6 路；等待主机许可不计入单次 HTTP 超时。批量 JoinSet 不再因单个 JoinError 提前返回；所有缺失结果在收尾阶段转换为失败并写入任务终态。`TaskTracker` 在最后一个源完成时保存总耗时，读取已结束任务时不再重算。

### 图标资源

生成一个无文字、透明背景、适合小尺寸的应用标记，保存 PNG 主资源并派生多尺寸 ICO。通过构建脚本嵌入 Windows EXE，通过 `ViewportBuilder` 设置窗口图标，安装脚本使用同一 ICO。侧栏和主要按钮使用 egui 可稳定显示的简洁符号，保持现有克制的工具型界面风格。

### 测试策略

1. 数据库测试覆盖多个订阅、未分组、筛选条件和 `real_proxy` 可用统计。
2. 数据库测试覆盖分组测速 ID 不越过订阅边界。
3. 运行格式检查、编译检查和完整测试。
4. 启动 GUI 验证订阅 Tab、计数、订阅测速、代理页两级选择和应用图标。

## 2026-08-31 节点发布平台设计

发布平台位于独立 publisher-server crate，使用 Axum、SQLx、SQLite 和 React。Web 会话使用 HttpOnly Cookie 和服务端随机会话记录；密码使用 Argon2。推送接口使用独立 Bearer Token。

节点推送采用事务性全量版本替换。订阅拉取根据用户订阅的协议、标签和数量限制动态生成 Base64 或 Clash 内容。每日额度通过 SQLite 条件更新原子扣减，成功生成内容后才计数。Caddy 负责发布域名（真实主机名只写在这台机器的配置里，不进仓库）的 HTTPS 和反向代理。
