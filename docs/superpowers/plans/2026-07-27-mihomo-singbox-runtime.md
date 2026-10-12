# Mihomo 与 Sing-box 双内核运行 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 根据节点协议自动选择 Mihomo 或 Sing-box，并让系统代理、TUN、分流、DNS、状态和真实测速使用同一实际内核。

**Architecture:** 复用现有 `KernelKind` 作为运行内核标识，新增纯函数能力矩阵和独立 `proxy/sing_box.rs` 配置编译器。`ProxyManager` 保持统一状态机，根据选择结果生成对应配置和启动命令；测速器按同一选择器分组调用内核探测器。

**Tech Stack:** Rust、Tokio、serde_json、Mihomo、Sing-box 1.13.14、egui、SQLx。

---

### Task 1: 自动内核选择器与状态

**Files:**
- Modify: `src/kernel/mod.rs`
- Modify: `src/proxy/mod.rs`
- Modify: `src/gui/mod.rs`
- Modify: `src/api/mod.rs`

- [ ] 添加 `KernelKind::supports_proxy_protocol` 和 `preferred_proxy_kernel(protocol)` 表驱动测试。
- [ ] `select_proxy_kernel(protocol)` 根据首选、安装状态和兼容性回退，Xray 不参与本阶段运行选择。
- [ ] `ProxyRuntimeStatus` 增加 `kernel: Option<KernelKind>`，停止状态为空，启动后返回实际内核。
- [ ] GUI 状态、日志和 API JSON 使用实际内核名，不再固定写 Mihomo。

### Task 2: Sing-box 节点配置编译器

**Files:**
- Create: `src/proxy/sing_box.rs`
- Modify: `src/proxy/mod.rs`
- Test: `src/proxy/sing_box.rs`

- [ ] 实现 `build_sing_box_config(node, options)`，生成 mixed 入站、proxy/direct/block outbound 和 route final。
- [ ] 支持 Hysteria2、TUIC、AnyTLS、WireGuard，并为每种协议添加真实字段形状测试。
- [ ] TLS 统一映射 `server_name`、`insecure`、`alpn`；Hysteria2 映射 salamander obfs；TUIC 映射 uuid/password/congestion_control。
- [ ] System 模式只启用 mixed 入站；TUN 模式额外启用 TUN 入站、`auto_route`、`strict_route` 和 DNS hijack 规则。
- [ ] HTTPS DNS 使用 Sing-box 1.13 server 对象格式。
- [ ] `TrafficRule` 编译为 route rule；Geo 只接受 `cn`、`CN` 和 `category-ads-all`，映射官方远程 `.srs` Rule Set。
- [ ] 使用 `sing-box check -c <config>` 验证测试配置。

### Task 3: ProxyManager 双内核进程

**Files:**
- Modify: `src/proxy/mod.rs`
- Modify: `src/main.rs`
- Test: `src/proxy/mod.rs`

- [ ] 工作目录配置文件按内核使用 `config.yaml` 或 `config.json`。
- [ ] 启动前调用自动选择器并写入状态内核。
- [ ] Mihomo 使用现有 `-d/-f` 命令；Sing-box 使用 `run -D <dir> -c <config>`。
- [ ] Mihomo 等待控制端口；Sing-box 等待 mixed 端口并检查子进程未提前退出。
- [ ] 两种内核均通过 mixed 端口访问 Google 204，失败时带内核名和最后日志行。
- [ ] 停止、异常退出、系统代理恢复和工作目录清理使用实际内核名。
- [ ] TUN 提权启动参数携带自动选择所需节点信息，不持久化错误的固定内核。

### Task 4: Sing-box 真实测速探测器

**Files:**
- Create: `src/sing_box_probe.rs`
- Modify: `src/main.rs`
- Modify: `src/speedtest/mod.rs`
- Test: `src/speedtest/mod.rs`

- [ ] 批量节点按 `select_proxy_kernel` 分为 Mihomo 和 Sing-box 两组。
- [ ] Mihomo 组复用现有批量 delay API。
- [ ] Sing-box 组生成多 outbound 配置并通过 Clash API delay 端点逐节点访问 Google 204；若 1.13 API 不返回单 outbound delay，则使用有界并发的临时本地入站探测。
- [ ] 结果分别写入 `latency_kernel=mihomo|sing_box` 和 `latency_test_kind=real_proxy`。
- [ ] 单节点测速继续复用同一分组管线。

### Task 5: 验证

- [ ] `cargo fmt`
- [ ] `cargo check --offline`
- [ ] `cargo test --offline`
- [ ] 使用 Mihomo 首选协议启动系统代理并验证 Google。
- [ ] 使用 Hysteria2/TUIC/AnyTLS 节点自动选择 Sing-box并验证状态、日志和 Google。
- [ ] 以管理员权限验证 Sing-box TUN、DNS 和分流。
- [ ] 切换跨内核节点，确认旧进程退出且系统代理不中断残留。
