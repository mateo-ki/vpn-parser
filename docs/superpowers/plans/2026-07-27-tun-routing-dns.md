# TUN、规则分流与 HTTPS DNS 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Mihomo 常驻代理基础上增加系统代理/TUN 双模式、可排序流量规则、GeoSite/GeoIP 与 HTTPS DNS 配置。

**Architecture:** 新增独立的 `proxy::config` 配置模型，负责校验并生成 Mihomo 原生配置片段；`ProxyManager` 持有并更新一份完整运行配置，API 与 GUI 共享该配置。TUN 模式不修改 Windows 系统代理，启动前检查管理员权限并返回明确错误。

**Tech Stack:** Rust、Serde、Serde JSON/YAML、Axum、egui、Tokio、Mihomo configuration schema。

---

### Task 1: 策略配置模型

**Files:**
- Create: `src/proxy/config.rs`
- Modify: `src/proxy/mod.rs`

- [x] 先写失败测试，覆盖 TUN 默认参数、DoH/fake-ip 配置、规则顺序和三种动作。
- [x] 运行 `cargo test --offline proxy::config::tests`，确认测试因类型或实现缺失而失败。
- [x] 实现 `TunConfig`、`DnsConfig`、`TrafficRule`、`RuleMatcher`、`RuleAction` 和校验逻辑。
- [x] 再次运行定向测试并确认通过。

### Task 2: Mihomo 配置生成

**Files:**
- Modify: `src/proxy/mod.rs`

- [x] 先写失败测试，覆盖 System/TUN 配置差异、Geo 规则、IP-CIDR、端口、进程、网络和 QUIC 复合规则。
- [x] 运行定向测试并确认预期失败。
- [x] 将 `MihomoConfigOptions` 改为可克隆的完整配置，并让 `build_mihomo_config` 输出 TUN、DNS 和有序规则。
- [x] 保证所有用户规则后追加 `MATCH,PROXY`，TUN 模式不启用系统代理。
- [x] 运行 `cargo test --offline proxy::tests` 并确认通过。

### Task 3: 运行时配置与 API

**Files:**
- Modify: `src/proxy/mod.rs`
- Modify: `src/api/mod.rs`

- [x] 先写失败测试，覆盖停止状态可更新配置、运行状态拒绝热修改。
- [x] 为 `ProxyManager` 增加读取和更新配置方法。
- [x] 新增 `GET/PUT /api/proxy/config`，复用 Serde 请求体并返回校验错误。
- [x] 运行代理与 API 相关测试。

### Task 4: GUI 双模式与策略设置

**Files:**
- Modify: `src/gui/mod.rs`

- [x] 在代理页增加系统代理/TUN 分段选择。
- [x] 增加 TUN 栈、DNS 开关、DoH 地址、fake-ip 和预置分流规则控件。
- [x] 配置变更写入 `ProxyManager`，运行期间禁用会改变启动配置的控件。
- [x] 保证状态、错误和日志区域布局稳定。

### Task 5: Windows TUN 权限边界

**Files:**
- Create: `src/platform/mod.rs`
- Modify: `src/main.rs`
- Modify: `src/proxy/mod.rs`

- [x] 测试 Windows 提权重启参数的转义规则。
- [x] Windows 使用系统令牌检测管理员身份，非 Windows 返回明确不支持信息。
- [x] TUN 启动时触发 UAC，并由提权实例恢复配置和节点后继续启动。

### Task 6: 文档与验证

**Files:**
- Modify: `requirements.md`
- Modify: `design.md`
- Modify: `tasks.md`

- [x] 更新阶段范围、配置模型、API 和已完成任务。
- [x] 运行 `cargo fmt`。
- [x] 运行 `cargo check --offline`。
- [x] 运行 `cargo test --offline`。
- [x] 运行 `cargo build --offline`。
