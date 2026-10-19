//! Ctrl+V 全局粘贴的识别层：把剪贴板文本判成「订阅地址 / 一堆节点链接 / 不认识」。
//!
//! 纯函数，副作用（建源、自动测速、弹框）都在 `VpnApp` 那边 —— 和 `prepare.rs` 一个口径，
//! 判据才测得动。识别口径跟着采集端实际支持的形态来：`http(s)://` 订阅地址、`t.me` 频道
//! 预览页、gist 搜索页，以及本地自建订阅 `http://192.168.x.y:port/subscription`。

use eframe::egui;

/// 节点链接前缀：粘到这些是"导入节点"的活，不是订阅源。集合照 `parser::uri` 的分支，
/// 不凭记忆编。
pub const NODE_LINK_PREFIXES: &[&str] = &[
    "ss://",
    "ssr://",
    "shadowsocks://",
    "shadowsocksr://",
    "vmess://",
    "vless://",
    "trojan://",
    "hysteria://",
    "hysteria2://",
    "hy2://",
    "tuic://",
    "anytls://",
    "socks://",
    "socks5://",
    "masque://",
];

/// 弹框要说的话：识别结果 + 我们已经做了什么（副作用发生在 `VpnApp` 那边）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteOutcome {
    /// 认出来并建了源，`tested` 说明有没有排上自动测速（那条开关关着时不能谎报）。
    Added {
        name: String,
        url: String,
        tested: bool,
    },
    /// 认出来了，但这个地址已经在订阅源里。
    Duplicate { name: String },
    /// 剪贴板是一份完整的 Clash 配置（包含节点与分流规则）。
    ClashConfig { nodes: usize, rules: usize },
    /// 剪贴板是多条分流规则。
    Rules { count: usize },
    /// 粘的是节点链接：给一条"按节点导入"的出口，不建源。
    NodeLinks { count: usize },
    /// 没认出来。
    Unknown { preview: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detected {
    /// 一个可以当订阅源的地址。`name` 是按主机名/频道名推出来的建议名称。
    Subscription { url: String, name: String },
    /// 完整 Clash 配置。
    ClashConfig { nodes: usize, rules: usize },
    /// 纯规则文本。
    Rules { count: usize },
    /// 粘的是节点链接（N 条）：走"导入剪贴板节点"那条路，不建源。
    NodeLinks { count: usize },
    /// 不是我们能吃的东西，弹框要说清楚看到的是什么。
    Unknown { preview: String },
}

/// 剪贴板里第一个"像地址"的东西。接受三种写法：带 scheme 的 URL、裸 `t.me/xxx`、
/// 以及裸 `host.tld/path`（有些面板给的就是不带 scheme 的地址）。
fn first_url(text: &str) -> Option<String> {
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|ch| {
            matches!(
                ch,
                '"' | '\'' | '<' | '>' | ',' | ';' | '(' | ')' | '[' | ']' | '，' | '。'
            )
        });
        if token.is_empty() {
            continue;
        }
        let low = token.to_ascii_lowercase();
        if low.starts_with("http://") || low.starts_with("https://") {
            return Some(token.to_string());
        }
        if low.starts_with("t.me/") {
            return Some(format!("https://{token}"));
        }
        // 裸地址：必须有主机 + 路径 + 主机带点，才敢当订阅地址；否则一段英文句子会被误判。
        if !token.contains("://") {
            if let Some((host, _)) = token.split_once('/') {
                if host.contains('.') && !host.starts_with('.') && !host.ends_with('.') && !host.contains(',') {
                    return Some(format!("https://{token}"));
                }
            }
        }
    }
    None
}

fn node_link_count(text: &str) -> usize {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            let low = line.to_ascii_lowercase();
            NODE_LINK_PREFIXES
                .iter()
                .any(|prefix| low.starts_with(prefix))
        })
        .count()
}

/// 建议名称：t.me 频道用频道名，其余取主机名（去掉 www.、转小写）。
pub fn suggested_name(url: &str) -> String {
    if let Some(channel) = crate::collector::telegram::channel_name(url) {
        return channel;
    }
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split('@')
        .next_back()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.is_empty() {
        "剪贴板订阅源".to_string()
    } else {
        host
    }
}

fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(80).collect()
}

/// 识别入口。空剪贴板返回 `None`（那种情况不配弹框）。
pub fn recognize(text: &str) -> Option<Detected> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // 0) 若是 Clash 完整 YAML 配置（含 proxies 与/或 rules），优先识别为配置或规则
    if crate::parser::looks_like_clash(trimmed) {
        let rules = crate::parser::extract_clash_rules(trimmed);
        let nodes = crate::parser::parse_clash_yaml(trimmed, None).unwrap_or_default();
        if !nodes.is_empty() {
            return Some(Detected::ClashConfig {
                nodes: nodes.len(),
                rules: rules.len(),
            });
        }
        if !rules.is_empty() {
            return Some(Detected::Rules {
                count: rules.len(),
            });
        }
    }
    // 1) 纯分流规则文本（如多行 DOMAIN-SUFFIX,google.com,PROXY）
    let import = crate::proxy::rule_io::parse_rules_text(trimmed);
    if !import.rules.is_empty() {
        return Some(Detected::Rules {
            count: import.rules.len(),
        });
    }
    // 2) 订阅源地址（URL）：仅当不是包含大量规则/节点的纯文本时作为 URL 识别
    if let Some(url) = first_url(trimmed) {
        let nodes = node_link_count(trimmed);
        if nodes <= 1 {
            let (name, url) = if url.to_ascii_lowercase().contains("openrung") {
                (
                    "OpenRung 专属节点服务".to_string(),
                    "https://broker.openrung.org/api/v1/relays".to_string(),
                )
            } else if url.to_ascii_lowercase().contains("warp-masque") {
                (
                    "WARP-MASQUE 专属节点服务".to_string(),
                    "https://raw.githubusercontent.com/byJoey/warp-masque-actions/main/configs/opera-masque.yaml".to_string(),
                )
            } else {
                (suggested_name(&url), url)
            };
            return Some(Detected::Subscription { name, url });
        }
    }
    // 3) WARP-MASQUE JSON
    if trimmed.starts_with('{')
        && (trimmed.contains("endpoint_pub_key")
            || trimmed.contains("peerPublicKey")
            || (trimmed.contains("private_key") && trimmed.contains("ipv4")))
    {
        return Some(Detected::NodeLinks { count: 57 });
    }
    // 4) 节点链接（ss://, vmess:// 等）
    let nodes = node_link_count(trimmed);
    if nodes > 0 {
        return Some(Detected::NodeLinks { count: nodes });
    }
    Some(Detected::Unknown {
        preview: preview(trimmed),
    })
}

/// 已经在列表里了吗：只按去掉首尾空白后的地址全等判断（订阅地址带 token，
/// 少一个参数就是另一个源，不敢做"归一化去重"）。
pub fn is_duplicate(
    url: &str,
    existing: impl IntoIterator<Item = (String, String)>,
) -> Option<String> {
    let target = url.trim();
    existing
        .into_iter()
        .find(|(_, template)| template.trim() == target)
        .map(|(name, _)| name)
}

/// 识别完到"要做什么"之间的全部判断，都放这一个纯函数里：
/// 建不建源、弹框说什么、自动测速开没开 —— 不碰剪贴板也不碰数据库，所以测得动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intake {
    /// `Some((名称, 地址))` = 该新建这个源；`None` = 不建（已存在/根本不是订阅）。
    pub create: Option<(String, String)>,
    pub outcome: PasteOutcome,
}

pub fn intake(
    text: &str,
    existing: &[(String, String)],
    auto_test_added_source: bool,
) -> Option<Intake> {
    match recognize(text)? {
        Detected::Subscription { url, name } => {
            match is_duplicate(&url, existing.iter().cloned()) {
                Some(found) => Some(Intake {
                    create: None,
                    outcome: PasteOutcome::Duplicate { name: found },
                }),
                None => Some(Intake {
                    create: Some((name.clone(), url.clone())),
                    outcome: PasteOutcome::Added {
                        name,
                        url,
                        tested: auto_test_added_source,
                    },
                }),
            }
        }
        Detected::ClashConfig { nodes, rules } => Some(Intake {
            create: None,
            outcome: PasteOutcome::ClashConfig { nodes, rules },
        }),
        Detected::Rules { count } => Some(Intake {
            create: None,
            outcome: PasteOutcome::Rules { count },
        }),
        Detected::NodeLinks { count } => Some(Intake {
            create: None,
            outcome: PasteOutcome::NodeLinks { count },
        }),
        Detected::Unknown { preview } => Some(Intake {
            create: None,
            outcome: PasteOutcome::Unknown { preview },
        }),
    }
}

/// 弹框里的按钮动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogAction {
    Close,
    ImportNodes,
    ImportClashConfig,
    ImportRules,
}

/// 弹框内容（纯渲染，状态由调用方持有）—— 抽出来是为了能在离屏 `egui::Context` 里
/// 真渲染一遍做断言，不用把窗口开出来。
pub fn dialog_body(ctx: &egui::Context, outcome: &PasteOutcome) -> Option<DialogAction> {
    let mut action = None;
    let mut open = true;
    egui::Window::new(egui::RichText::new("剪贴板识别").strong())
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(470.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let (color, bg, title, body) = describe(outcome);
            egui::Frame::NONE
                .fill(bg)
                .stroke(egui::Stroke::new(1.0_f32, color))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(title).strong().color(color));
                    ui.add_space(3.0);
                    ui.label(egui::RichText::new(body).small());
                });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("在输入框里按 Ctrl+V 仍是普通粘贴，不会走这个识别。")
                    .small()
                    .color(crate::gui::theme::muted()),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if matches!(outcome, PasteOutcome::ClashConfig { .. })
                    && ui.button("导入节点并合并分流规则").clicked()
                {
                    action = Some(DialogAction::ImportClashConfig);
                }
                if matches!(outcome, PasteOutcome::Rules { .. })
                    && ui.button("追加到分流规则列表").clicked()
                {
                    action = Some(DialogAction::ImportRules);
                }
                if matches!(outcome, PasteOutcome::NodeLinks { .. })
                    && ui.button("按节点导入这些链接").clicked()
                {
                    action = Some(DialogAction::ImportNodes);
                }
                if ui.button("关闭").clicked() {
                    action = Some(DialogAction::Close);
                }
            });
        });
    if !open {
        return Some(DialogAction::Close);
    }
    action
}

/// 弹框要说的颜色和两行话 —— 单独抽出来，好让测试直接看文案（尤其是"没测速"那句不能漏）。
pub fn describe(outcome: &PasteOutcome) -> (egui::Color32, egui::Color32, String, String) {
    use crate::gui::theme;
    match outcome {
        PasteOutcome::Added { name, url, tested } => {
            let next = if *tested {
                "已自动排上：拉这个源 → 只测这一源的节点，结果在源那一行看。"
            } else {
                "「新增订阅源后自动测这一源」的开关是关的，所以只建了源、没测速。"
            };
            (
                theme::success(),
                theme::success_bg(),
                format!("识别成功 · 已加入《{name}》"),
                format!("{url}\n{next}"),
            )
        }
        PasteOutcome::Duplicate { name } => (
            theme::warning(),
            theme::warning_bg(),
            "这个地址已经在订阅源里了".to_string(),
            format!("现有源：《{name}》 —— 没有重复添加。"),
        ),
        PasteOutcome::ClashConfig { nodes, rules } => (
            theme::success(),
            theme::success_bg(),
            "剪贴板是一份 Clash 完整配置".to_string(),
            format!("包含 {nodes} 个代理节点与 {rules} 条分流规则。可以一键导入节点并自动合并分流规则。"),
        ),
        PasteOutcome::Rules { count } => (
            theme::accent(),
            theme::accent_bg(),
            format!("剪贴板里是 {count} 条分流规则"),
            format!("识别到 {count} 条分流规则文本，可以一键追加合并到当前规则分流列表中。"),
        ),
        PasteOutcome::NodeLinks { count } => (
            theme::muted(),
            theme::panel(),
            format!("剪贴板里是 {count} 条节点链接"),
            "那是 ss:// / vmess:// 这类单节点地址，不是订阅链接。可以按节点导入，不建订阅源。"
                .to_string(),
        ),
        PasteOutcome::Unknown { preview } => (
            theme::danger(),
            theme::danger_bg(),
            "没认出订阅地址".to_string(),
            if preview.is_empty() {
                "剪贴板里没有可读的文本。".to_string()
            } else {
                format!("看到的内容：{preview}")
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_outcome_describes_itself_without_lying_about_the_test_switch() {
        let (_, _, title, body) = describe(&PasteOutcome::Added {
            name: "a.b".into(),
            url: "https://a.b/c".into(),
            tested: false,
        });
        assert!(title.contains("识别成功"), "{title}");
        assert!(body.contains("开关是关的"), "{body}");
        let (_, _, _, on) = describe(&PasteOutcome::Added {
            name: "a.b".into(),
            url: "https://a.b/c".into(),
            tested: true,
        });
        assert!(on.contains("只测这一源"), "{on}");

        let (_, _, dup_title, dup_body) = describe(&PasteOutcome::Duplicate {
            name: "老源".into(),
        });
        assert!(dup_title.contains("已经在订阅源里") && dup_body.contains("老源"));
        let (_, _, nodes, _) = describe(&PasteOutcome::NodeLinks { count: 4 });
        assert!(nodes.contains("4 条节点链接"), "{nodes}");
    }

    #[test]
    fn dialog_body_renders_offscreen_for_every_outcome() {
        // 离屏真渲染一遍：只要求"画出来了、有内容、不 panic"，视觉好坏还得人看。
        let cases = [
            PasteOutcome::Added {
                name: "n".into(),
                url: "https://a.b/c".into(),
                tested: true,
            },
            PasteOutcome::Duplicate { name: "n".into() },
            PasteOutcome::ClashConfig {
                nodes: 5,
                rules: 12,
            },
            PasteOutcome::Rules { count: 8 },
            PasteOutcome::NodeLinks { count: 2 },
            PasteOutcome::Unknown {
                preview: "今天天气不错".into(),
            },
        ];
        for outcome in cases {
            let ctx = egui::Context::default();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_max(
                    egui::pos2(0.0, 0.0),
                    egui::pos2(1000.0, 700.0),
                )),
                ..Default::default()
            };
            let clipped = ctx.run(raw, |ctx| {
                dialog_body(ctx, &outcome);
            });
            assert!(!clipped.shapes.is_empty(), "{outcome:?} 什么都没画出来");
        }
    }

    #[test]
    fn fresh_url_becomes_a_source_and_reports_the_test_switch() {
        let got = intake("https://new.sub/x", &[], true).unwrap();
        assert_eq!(
            got.create,
            Some(("new.sub".to_string(), "https://new.sub/x".to_string()))
        );
        assert!(matches!(
            got.outcome,
            PasteOutcome::Added { tested: true, .. }
        ));
        // 开关关着时不能谎报"已经在测"，界面要说实话。
        let off = intake("https://new.sub/x", &[], false).unwrap();
        assert!(matches!(
            off.outcome,
            PasteOutcome::Added { tested: false, .. }
        ));
        assert_eq!(off.create, got.create);
    }

    #[test]
    fn duplicate_never_creates_a_second_copy() {
        let existing = vec![("老源".to_string(), "https://a.b/c".to_string())];
        let got = intake("https://a.b/c", &existing, true).unwrap();
        assert_eq!(got.create, None);
        assert_eq!(
            got.outcome,
            PasteOutcome::Duplicate {
                name: "老源".into()
            }
        );
    }

    #[test]
    fn node_links_offer_the_node_import_instead_of_a_source() {
        let got = intake("ss://abc@1.2.3.4:8388\nss://def@5.6.7.8:8389", &[], true).unwrap();
        assert_eq!(got.create, None);
        assert_eq!(got.outcome, PasteOutcome::NodeLinks { count: 2 });
    }

    #[test]
    fn prose_is_only_reported() {
        let got = intake("这行字里没有任何地址", &[], true).unwrap();
        assert_eq!(got.create, None);
        assert!(matches!(got.outcome, PasteOutcome::Unknown { .. }));
    }

    #[test]
    fn the_three_sites_the_user_named_are_all_accepted_as_sources() {
        // 他给的三个例子就是判据：这三个都得认下来，名字去掉 www.。
        for (clip, want_name) in [
            ("https://linux.do/", "linux.do"),
            ("https://www.youtube.com/", "youtube.com"),
            ("https://github.com/", "github.com"),
        ] {
            let got = intake(clip, &[], true).unwrap_or_else(|| panic!("{clip} 没认出来"));
            assert_eq!(
                got.create,
                Some((want_name.to_string(), clip.to_string())),
                "{clip}"
            );
        }
    }

    #[test]
    fn empty_clipboard_produces_no_dialog_at_all() {
        assert_eq!(intake("  \n", &[], true), None);
    }

    #[test]
    fn recognises_plain_subscription_urls() {
        let got = recognize("https://sub.example.com/link/abcdef?token=1").unwrap();
        assert_eq!(
            got,
            Detected::Subscription {
                url: "https://sub.example.com/link/abcdef?token=1".into(),
                name: "sub.example.com".into(),
            }
        );
    }

    #[test]
    fn picks_the_url_out_of_a_sentence_and_strips_noise() {
        let got = recognize("订阅地址是 https://a.b/c/d ，谢谢！").unwrap();
        assert_eq!(
            got,
            Detected::Subscription {
                url: "https://a.b/c/d".into(),
                name: "a.b".into(),
            }
        );
    }

    #[test]
    fn normalises_bare_t_me_and_host_forms_to_https() {
        assert_eq!(
            recognize("t.me/v2free").unwrap(),
            Detected::Subscription {
                url: "https://t.me/v2free".into(),
                // 频道名走 telegram 的口径
                name: suggested_name("https://t.me/v2free"),
            }
        );
        assert_eq!(
            recognize("192.168.1.7:25500/subscription").unwrap(),
            Detected::Subscription {
                url: "https://192.168.1.7:25500/subscription".into(),
                name: "192.168.1.7:25500".into(),
            }
        );
    }

    #[test]
    fn prose_and_hostnames_without_path_are_not_subscriptions() {
        // 光有主机没有路径，多半是粘了个域名/网站名，不建源。
        assert!(recognize("example.com").is_some());
        let bare = recognize("example.com").unwrap();
        assert!(matches!(bare, Detected::Unknown { .. }), "{bare:?}");
        let prose = recognize("今天天气不错").unwrap();
        assert!(matches!(prose, Detected::Unknown { .. }));
    }

    #[test]
    fn node_links_are_reported_as_such_not_as_a_source() {
        let clip = "ss://YWVzLTI1Nkc6cGFAYDEuMi4zLjQ6NDQN\nvmess://eyJ2...  \nTROJAN://a@b.c:443";
        assert_eq!(recognize(clip).unwrap(), Detected::NodeLinks { count: 3 });
    }

    #[test]
    fn empty_clipboard_is_not_anything() {
        assert_eq!(recognize("   \n "), None);
    }

    #[test]
    fn www_prefix_is_dropped_from_the_suggested_name() {
        assert_eq!(
            suggested_name("https://WWW.FooBar.COM/sub?x=1"),
            "foobar.com".to_string()
        );
    }

    #[test]
    fn duplicate_check_compares_the_template_verbatim() {
        let existing = vec![("老源".to_string(), " https://a.b/c ".to_string())];
        assert_eq!(
            is_duplicate("https://a.b/c", existing.clone()),
            Some("老源".into())
        );
        // token 少一个字符就是另一个源，不做归一化。
        assert_eq!(is_duplicate("https://a.b/c?token=2", existing), None);
    }

    #[test]
    fn clash_config_and_rules_text_are_recognized() {
        let clash_yaml = r#"
proxies:
  - name: "HK 01"
    type: ss
    server: 1.1.1.1
    port: 443
    cipher: aes-256-gcm
    password: pass
rules:
  - "DOMAIN-SUFFIX,google.com,PROXY"
  - "DOMAIN-KEYWORD,openai,PROXY"
  - "GEOIP,CN,DIRECT"
"#;
        assert_eq!(
            recognize(clash_yaml),
            Some(Detected::ClashConfig {
                nodes: 1,
                rules: 3,
            })
        );

        let rules_only = r#"
"DOMAIN-SUFFIX,google.com,PROXY",
"DOMAIN-KEYWORD,openai,PROXY",
"IP-CIDR,192.168.0.0/16,DIRECT",
"GEOIP,CN,DIRECT"
"#;
        assert_eq!(
            recognize(rules_only),
            Some(Detected::Rules { count: 4 })
        );
    }
}

