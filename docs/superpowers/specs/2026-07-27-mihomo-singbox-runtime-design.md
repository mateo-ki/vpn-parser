# Mihomo 与 Sing-box 双内核运行设计

## 目标

在保留 Mihomo 现有代理能力的基础上，增加由项目自行维护的 Sing-box 运行时，并根据节点协议自动选择实际代理内核。系统代理、TUN、DNS、分流、日志和健康检查均返回真实使用的内核。

## 架构

新增内核无关的代理运行层：

- `ProxyKernel`：`Mihomo`、`SingBox`。
- `ProxyConfigOptions`：保存系统代理/TUN、监听端口、DNS 和有序分流规则。
- `KernelSelector`：根据节点协议和已安装内核选择运行内核。
- `MihomoRuntime`：生成 YAML、启动进程、检查控制端口、读取日志。
- `SingBoxRuntime`：生成原生 JSON、启动进程、检查入站端口、读取日志。
- `ProxyManager`：统一状态机，负责启动、停止、重启、系统代理快照恢复和 TUN 提权交接。

优先使用枚举分派保持实现直接，不引入不必要的动态插件系统。

## 自动选择规则

- `ssr`、`hysteria`：Mihomo。
- `hysteria2`、`hy2`、`tuic`、`anytls`、`wireguard`、`wg`：Sing-box。
- `vless`、`vmess`、`trojan`、`ss`、`shadowsocks`、`http`、`https`、`socks`、`socks5`：优先 Mihomo。
- 首选内核未安装时，如果另一已安装内核支持该协议则自动回退到兼容内核。
- 两个内核都不支持或都未安装时返回结构化错误，不启动进程。

## 配置编译

- 现有 Mihomo 配置生成行为保持兼容。
- Sing-box 使用原生 JSON，包含 mixed 入站、目标节点 outbound、direct/block outbound、DNS 和 route rules。
- 现有 `TrafficRule` 作为内核无关规则模型，分别编译为 Mihomo rule 字符串和 Sing-box route rule 对象。
- Sing-box 1.13 的 Geo 规则使用官方 `.srs` Rule Set；`geosite:cn`、`geosite:category-ads-all` 和 `geoip:CN` 映射为对应远程规则集，其他未知 Geo 分类返回明确的不支持错误。
- `MATCH,PROXY` 只属于 Mihomo 输出；Sing-box 使用最终规则或 `final` 指向代理 outbound。
- 系统代理模式监听本机 mixed 端口。
- Sing-box TUN 使用其原生 TUN inbound、auto_route 和 DNS hijack 能力，不复用 Mihomo YAML 字段。

## 状态与 API

- `ProxyRuntimeStatus` 增加 `kernel`。
- 启动、重启和状态 API 返回实际内核。
- GUI 在运行状态、当前节点和日志区域显示 Mihomo 或 Sing-box。
- 节点切换导致内核变化时，先停止并清理旧内核，再启动新内核。
- 内核更新时禁止替换当前正在运行的对应可执行文件。

## 健康检查与错误

- 两种运行时启动后都通过本地 mixed 代理访问 Google 204。
- `dial tcp <节点入口>:<端口> timeout` 分类为入口连接超时，不误报为 DNS 或分流错误。
- 配置不兼容、内核缺失、进程提前退出、入站端口未就绪和真实代理验证失败分别返回明确错误。
- 启动失败时恢复系统代理快照并清理子进程与临时配置。

## 协议边界

- 本阶段不实现 Xray 运行时。
- Xray 继续由内核管理模块维护，用于后续兼容扩展。
- 自动选择基于项目已实现的配置转换能力，而不是内核理论上支持的全部协议。

## 测试

- 能力矩阵和自动选择规则表驱动测试。
- Mihomo 与 Sing-box 配置快照测试。
- 系统代理、TUN、DNS 和分流分别验证关键字段。
- 内核切换时旧进程停止、状态内核更新。
- 首选内核缺失时兼容回退；不兼容时拒绝启动。
- 两种内核都执行真实代理健康检查和失败清理。

## 实施阶段

1. 抽取内核无关配置和选择器，保持 Mihomo 行为不变。
2. 实现 Sing-box 系统代理运行。
3. 实现 Sing-box 分流与 HTTPS DNS。
4. 实现 Sing-box TUN 和提权交接。
5. 接入按内核分组测速。
