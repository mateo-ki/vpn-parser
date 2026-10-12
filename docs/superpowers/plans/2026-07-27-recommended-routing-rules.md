# 推荐分流规则 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将“推荐分流”升级为可确认覆盖的国内直连、广告拒绝、其余代理默认规则集。

**Architecture:** 在 `src/proxy/config.rs` 提供无状态的推荐规则构建函数，使规则内容和顺序可以独立测试。`src/gui/mod.rs` 只维护确认弹窗状态，用户确认后整体替换 `proxy_config.rules` 并调用现有异步保存流程；`build_mihomo_config` 保持负责校验规则、启用 Geo 数据并追加 `MATCH,PROXY`。

**Tech Stack:** Rust、egui/eframe、Serde、serde_json、Tokio、Cargo 内置测试。

---

## 文件结构

- Modify: `src/proxy/config.rs` - 定义推荐规则纯函数及其单元测试。
- Modify: `src/proxy/mod.rs` - 扩展 Mihomo 配置集成测试；现有 `pub use config::*` 会自动导出推荐规则函数。
- Modify: `src/gui/mod.rs` - 增加覆盖确认状态、说明文字和确认对话框。
- Verify: `docs/superpowers/specs/2026-07-27-recommended-routing-rules-design.md` - 对照已确认规格验收，不再改动需求范围。

由于当前仓库没有首个 Git 提交且用户未要求提交，本计划以测试检查点替代提交步骤。

### Task 1: 推荐规则构建函数

**Files:**
- Modify: `src/proxy/config.rs:139`
- Test: `src/proxy/config.rs:198`

- [ ] **Step 1: 先写失败的规则顺序测试**

在 `src/proxy/config.rs` 的测试模块新增：

```rust
#[test]
fn recommended_rules_cover_lan_ads_cn_and_quic_in_order() {
    let rendered = recommended_traffic_rules()
        .iter()
        .map(TrafficRule::to_mihomo_rule)
        .collect::<Result<Vec<_>>>()
        .unwrap();

    assert_eq!(
        rendered,
        vec![
            "DOMAIN,localhost,DIRECT",
            "DOMAIN-SUFFIX,lan,DIRECT",
            "DOMAIN-SUFFIX,home.arpa,DIRECT",
            "IP-CIDR,127.0.0.0/8,DIRECT",
            "IP-CIDR,10.0.0.0/8,DIRECT",
            "IP-CIDR,172.16.0.0/12,DIRECT",
            "IP-CIDR,192.168.0.0/16,DIRECT",
            "IP-CIDR,169.254.0.0/16,DIRECT",
            "GEOSITE,category-ads-all,REJECT",
            "GEOSITE,cn,DIRECT",
            "GEOIP,CN,DIRECT",
            "AND,((NETWORK,udp),(DST-PORT,443)),REJECT",
        ]
    );
}
```

- [ ] **Step 2: 运行测试并确认函数尚不存在**

Run: `cargo test --offline proxy::config::tests::recommended_rules_cover_lan_ads_cn_and_quic_in_order`

Expected: FAIL，错误包含 `cannot find function recommended_traffic_rules`。

- [ ] **Step 3: 实现最小纯函数**

在 `TrafficRule` 实现之后新增：

```rust
pub fn recommended_traffic_rules() -> Vec<TrafficRule> {
    use RuleAction::{Direct, Reject};

    vec![
        TrafficRule { matcher: RuleMatcher::Domain("localhost".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::DomainSuffix("lan".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::DomainSuffix("home.arpa".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::IpCidr("127.0.0.0/8".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::IpCidr("10.0.0.0/8".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::IpCidr("172.16.0.0/12".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::IpCidr("192.168.0.0/16".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::IpCidr("169.254.0.0/16".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::GeoSite("category-ads-all".into()), action: Reject },
        TrafficRule { matcher: RuleMatcher::GeoSite("cn".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::GeoIp("CN".into()), action: Direct },
        TrafficRule { matcher: RuleMatcher::Quic, action: Reject },
    ]
}
```

- [ ] **Step 4: 运行配置模块测试**

Run: `cargo test --offline proxy::config::tests`

Expected: PASS，推荐规则测试和现有匹配器测试全部通过。

### Task 2: Mihomo 配置集成覆盖

**Files:**
- Test: `src/proxy/mod.rs:715`

- [ ] **Step 1: 写完整配置测试**

在 `src/proxy/mod.rs` 测试模块新增：

```rust
#[test]
fn recommended_routing_enables_geo_and_keeps_proxy_fallback_last() {
    let selected = node(
        "vless",
        json!({
            "server": "1.2.3.4",
            "port": 443,
            "uuid": "11111111-1111-1111-1111-111111111111"
        }),
    );
    let options = MihomoConfigOptions {
        rules: recommended_traffic_rules(),
        ..MihomoConfigOptions::default()
    };

    let config = build_mihomo_config(&selected, options).unwrap();
    let rules = config["rules"].as_array().unwrap();

    assert_eq!(config["geodata-mode"], true);
    assert_eq!(rules.len(), 13);
    assert_eq!(rules[8], "GEOSITE,category-ads-all,REJECT");
    assert_eq!(rules[9], "GEOSITE,cn,DIRECT");
    assert_eq!(rules.last().unwrap(), "MATCH,PROXY");
}
```

- [ ] **Step 2: 运行集成测试**

Run: `cargo test --offline proxy::tests::recommended_routing_enables_geo_and_keeps_proxy_fallback_last`

Expected: PASS，Geo 模式启用且 `MATCH,PROXY` 位于最后。

### Task 3: GUI 覆盖确认交互

**Files:**
- Modify: `src/gui/mod.rs:139`
- Modify: `src/gui/mod.rs:205`
- Modify: `src/gui/mod.rs:1680`

- [ ] **Step 1: 引入推荐规则函数并增加状态字段**

在代理模块导入列表加入 `recommended_traffic_rules`，在 `VpnApp` 增加并初始化：

```rust
show_recommended_rules_confirm: bool,
```

```rust
show_recommended_rules_confirm: false,
```

- [ ] **Step 2: 调整按钮说明和点击行为**

在新增规则操作行中保留“推荐分流”按钮，并为它增加悬停说明：

```rust
add_recommended = ui
    .button("推荐分流")
    .on_hover_text("覆盖为国内直连、广告拒绝、其余代理")
    .clicked();
```

按钮下方增加常驻说明：

```rust
ui.label(
    RichText::new("推荐规则：局域网与国内直连，广告和 QUIC 拒绝，其余代理")
        .small()
        .color(MUTED),
);
```

将旧的 `rules.extend(...)` 替换为：

```rust
if add_recommended {
    self.show_recommended_rules_confirm = true;
}
```

- [ ] **Step 3: 添加确认对话框**

在 `ui_proxy_rules` 尾部调用独立方法：

```rust
self.ui_recommended_rules_confirm(ui.ctx());
```

并在 `impl VpnApp` 中新增真正阻断背景输入的模态确认框：

```rust
fn ui_recommended_rules_confirm(&mut self, ctx: &egui::Context) {
    if !self.show_recommended_rules_confirm {
        return;
    }

    let modal = egui::Modal::new(egui::Id::new("recommended_rules_confirm")).show(ctx, |ui| {
        ui.heading("覆盖分流规则");
        ui.label("现有分流规则将被推荐规则覆盖。此操作保存后生效。");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("取消").clicked() {
                Some(false)
            } else if ui.add(primary_btn("确认覆盖")).clicked() {
                Some(true)
            } else {
                None
            }
        })
        .inner
    });

    match modal.inner {
        Some(true) => {
            self.proxy_config.rules = recommended_traffic_rules();
            self.show_recommended_rules_confirm = false;
            self.save_proxy_config();
        }
        Some(false) => self.show_recommended_rules_confirm = false,
        None if modal.should_close() => self.show_recommended_rules_confirm = false,
        None => {}
    }
}
```

- [ ] **Step 4: 格式化并编译检查**

Run: `cargo fmt`

Expected: 命令成功，无格式差异错误。

Run: `cargo check --offline`

Expected: PASS，仅允许仓库现有的 `dead_code` 警告。

### Task 4: 完整验证

**Files:**
- Verify: `src/proxy/config.rs`
- Verify: `src/proxy/mod.rs`
- Verify: `src/gui/mod.rs`

- [ ] **Step 1: 运行完整测试**

Run: `cargo test --offline`

Expected: 所有测试通过，新增测试计入总数且无失败。

- [ ] **Step 2: 构建并启动桌面程序**

Run: `cargo build --offline`

Expected: PASS，生成 `target/debug/vpn-parser.exe`。

启动最新版程序，并保持当前代理节点不变。

- [ ] **Step 3: 验证取消不会修改规则**

1. 停止代理并进入“代理运行 > 分流规则”。
2. 记录当前第一条规则。
3. 点击“推荐分流”，确认弹窗说明覆盖行为。
4. 点击“取消”。
5. 验证规则数量和第一条规则均未变化。

- [ ] **Step 4: 验证确认覆盖和持久化**

1. 再次点击“推荐分流”并选择“确认覆盖”。
2. 验证界面显示 12 条推荐规则，第一条为 `DOMAIN · localhost`。
3. 验证广告规则位于国内 GeoSite 规则之前。
4. 重新读取代理配置，验证 12 条规则已持久化。
5. 启动代理，验证状态为“运行中”且 Google 显示“已连通”。

- [ ] **Step 5: 检查最终工作区**

Run: `git status --short`

Expected: 仅显示当前仓库已有未跟踪文件和本功能修改；不删除、不重置、不提交用户文件。
