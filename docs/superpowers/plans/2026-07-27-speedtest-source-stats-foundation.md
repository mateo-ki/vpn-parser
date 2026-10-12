# 测速语义与订阅统计基础 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除单节点 TCP 测速误判，记录真实代理测速元数据，并为每个订阅源提供准确的总数、可用、不可用和未测速统计。

**Architecture:** 先扩展节点测速结果的数据模型和 SQLite 字段，再让单节点测速复用批量真实代理管线，并修正 UDP 协议预筛。订阅源统计通过 `Database::list_sources` 的单次聚合查询产生，GUI 和 API 复用同一 `Source` 模型。

**Tech Stack:** Rust、Tokio、SQLx/SQLite、egui、Serde、Mihomo 真实代理探测。

---

### Task 1: 测速结果元数据

**Files:**
- Modify: `src/models/mod.rs`
- Modify: `src/db/mod.rs`
- Test: `src/db/mod.rs`

- [ ] **Step 1: 写数据库迁移与读写失败测试**

新增测试，创建临时数据库后写入真实代理结果，再读取节点并断言：

```rust
assert_eq!(node.latency_kernel.as_deref(), Some("mihomo"));
assert_eq!(node.latency_test_kind.as_deref(), Some("real_proxy"));
```

Run: `cargo test --offline db::tests::persists_speedtest_kernel_and_kind`

Expected: FAIL，`Node` 尚无对应字段或数据库没有对应列。

- [ ] **Step 2: 扩展节点模型**

在 `Node` 和 `NodeSummary` 增加：

```rust
pub latency_kernel: Option<String>,
pub latency_test_kind: Option<String>,
```

在 `LatencyResult` 增加同名字段，使 API 能返回本次验证使用的内核和类型。

- [ ] **Step 3: 增加数据库列和历史语义迁移**

在 `Database::migrate` 增加：

```rust
self.ensure_column("nodes", "latency_kernel", "TEXT").await?;
self.ensure_column("nodes", "latency_test_kind", "TEXT").await?;
```

已有 `latency_ok IS NOT NULL` 且测试类型为空的记录无法证明是真实代理结果，保持 `latency_test_kind = NULL`，由统计归入未测速。

- [ ] **Step 4: 扩展查询与映射**

所有构造 `Node`/`NodeSummary` 的 SELECT 增加：

```sql
latency_kernel, latency_test_kind
```

`row_to_node` 和节点摘要映射使用 `try_get` 读取两个可空字段，兼容迁移中的数据库。

- [ ] **Step 5: 让测速写入接口携带元数据**

将单条和批量更新项扩展为：

```rust
(id, ok, latency_ms, download_kbps, latency_kernel, latency_test_kind)
```

真实代理结果写入 `Some("mihomo")` 和 `Some("real_proxy")`；仅 TCP 探测写入 `None` 和 `Some("tcp_only")`。

- [ ] **Step 6: 运行数据库测试**

Run: `cargo test --offline db::tests`

Expected: PASS。

### Task 2: 单节点真实代理测速

**Files:**
- Modify: `src/speedtest/mod.rs`
- Modify: `src/gui/mod.rs`
- Modify: `src/api/mod.rs`
- Test: `src/speedtest/mod.rs`

- [ ] **Step 1: 写单节点复用真实管线测试**

将批量流程中“合并真实代理结果”的纯逻辑抽为函数，并测试真实结果覆盖 TCP 结果且写入：

```rust
assert!(result.ok);
assert_eq!(result.latency_kernel.as_deref(), Some("mihomo"));
assert_eq!(result.latency_test_kind.as_deref(), Some("real_proxy"));
```

- [ ] **Step 2: 重写 `SpeedTester::test_one`**

`test_one` 不再直接调用 `probe_full`，改为：

```rust
let mut summary = self.test_many(Some(vec![node_id.to_owned()]), opts).await?;
summary
    .results
    .pop()
    .ok_or_else(|| anyhow::anyhow!("node not found"))
```

这样 GUI 和 `POST /api/nodes/{id}/speedtest` 自动使用真实代理验证。

- [ ] **Step 3: 修正 UI 文案**

- 节点行按钮悬停说明改为“通过实际运行内核完成代理握手和联网验证”。
- 设置页说明删除“单节点测速仍是快速 TCP”。
- 进度和完成状态继续使用现有真实代理文案。

- [ ] **Step 4: 运行单节点相关测试**

Run: `cargo test --offline speedtest`

Expected: PASS。

### Task 3: UDP 协议绕过 TCP 预筛

**Files:**
- Modify: `src/speedtest/mod.rs`
- Test: `src/speedtest/mod.rs`

- [ ] **Step 1: 写协议表驱动失败测试**

```rust
#[test]
fn udp_proxy_protocols_skip_tcp_prefilter() {
    for protocol in ["hysteria", "hysteria2", "hy2", "tuic", "wireguard", "wg"] {
        assert!(requires_proxy_without_tcp(protocol), "{protocol}");
    }
    for protocol in ["vless", "vmess", "trojan", "ss"] {
        assert!(!requires_proxy_without_tcp(protocol), "{protocol}");
    }
}
```

- [ ] **Step 2: 扩展免预筛列表**

```rust
matches!(
    protocol.to_ascii_lowercase().as_str(),
    "hysteria" | "hysteria2" | "hy2" | "tuic" | "wireguard" | "wg"
)
```

- [ ] **Step 3: 运行目标测试**

Run: `cargo test --offline speedtest::tests::udp_proxy_protocols_skip_tcp_prefilter`

Expected: PASS。

### Task 4: 订阅源四态统计

**Files:**
- Modify: `src/models/mod.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/gui/mod.rs`
- Test: `src/db/mod.rs`

- [ ] **Step 1: 写三态聚合失败测试**

创建一个订阅源及四类节点：真实成功、真实失败、`tcp_only`、未测试，断言：

```rust
assert_eq!(source.current_node_count, 4);
assert_eq!(source.available_node_count, 1);
assert_eq!(source.unavailable_node_count, 1);
assert_eq!(source.untested_node_count, 2);
```

再创建空订阅源并断言四项均为零。

- [ ] **Step 2: 扩展 `Source` 模型和创建默认值**

```rust
pub current_node_count: i64,
pub available_node_count: i64,
pub unavailable_node_count: i64,
pub untested_node_count: i64,
```

`create_source` 新对象四项初始化为 `0`。

- [ ] **Step 3: 使用一次聚合查询实现 `list_sources`**

```sql
SELECT s.*,
       COALESCE(ns.total, 0) AS current_node_count,
       COALESCE(ns.available, 0) AS available_node_count,
       COALESCE(ns.unavailable, 0) AS unavailable_node_count,
       COALESCE(ns.untested, 0) AS untested_node_count
FROM sources s
LEFT JOIN (
    SELECT source_id,
           COUNT(*) AS total,
           SUM(CASE WHEN latency_ok = 1 AND latency_test_kind = 'real_proxy' THEN 1 ELSE 0 END) AS available,
           SUM(CASE WHEN latency_ok = 0 AND latency_test_kind = 'real_proxy' THEN 1 ELSE 0 END) AS unavailable,
           SUM(CASE WHEN latency_ok IS NULL OR latency_test_kind IS NULL OR latency_test_kind != 'real_proxy' THEN 1 ELSE 0 END) AS untested
    FROM nodes
    GROUP BY source_id
) ns ON ns.source_id = s.id
ORDER BY s.created_at DESC
```

`get_source` 使用相同聚合口径，避免创建/更新后字段缺失。

- [ ] **Step 4: 更新订阅源 GUI**

节点统计单元格固定显示：

```text
总 {current_node_count}
可用 {available_node_count} · 不可用 {unavailable_node_count}
未测速 {untested_node_count}
```

`last_node_count` 改为“最近解析”信息，避免与当前总数混淆。

- [ ] **Step 5: 运行统计测试**

Run: `cargo test --offline db::tests::source_statistics_split_real_results_and_untested`

Expected: PASS。

### Task 5: 推荐分流确认交互收尾

**Files:**
- Modify: `src/gui/mod.rs`
- Reference: `docs/superpowers/plans/2026-07-27-recommended-routing-rules.md`

- [ ] **Step 1: 接入已完成的推荐规则函数**

为 `VpnApp` 增加 `show_recommended_rules_confirm`，按钮只打开确认框，不直接修改规则。

- [ ] **Step 2: 使用 `egui::Modal` 覆盖确认**

确认后整体替换为 `recommended_traffic_rules()` 并调用 `save_proxy_config()`；取消、点击遮罩或按 Escape 均不修改规则。

- [ ] **Step 3: 编译检查**

Run: `cargo fmt`

Run: `cargo check --offline`

Expected: PASS，仅允许仓库已有 `dead_code` 警告。

### Task 6: 完整验证

**Files:**
- Verify: `src/models/mod.rs`
- Verify: `src/db/mod.rs`
- Verify: `src/speedtest/mod.rs`
- Verify: `src/gui/mod.rs`
- Verify: `src/api/mod.rs`

- [ ] **Step 1: 运行完整测试**

Run: `cargo test --offline`

Expected: 全部通过。

- [ ] **Step 2: 构建并启动桌面程序**

Run: `cargo build --offline`

- [ ] **Step 3: 桌面验收**

1. 单节点“测速”显示真实代理结果，不再只显示 TCP。
2. 订阅源列表显示四项统计且数值满足恒等式。
3. 推荐分流确认框阻断背景操作，取消不改规则，确认后显示 12 条推荐规则。
4. 启动已验证节点，Google 状态显示“已连通”。

- [ ] **Step 4: 工作区检查**

Run: `git status --short`

Expected: 不删除、不重置、不提交用户文件。
