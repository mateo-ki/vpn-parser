//! 分流规则的导入 / 导出。
//!
//! 入口是"一段文本"，不是"一个文件"：他从 Clash Verge 那种界面里是**复制**出来的，
//! 所以这里同时吃三种形状 ——
//! * JSON 数组：`["DOMAIN-SUFFIX,a.com,PROXY", ...]`
//! * JSON 对象：`{"rules":[...]}`、`{"prependRules":[...]}`（Clash Verge 扩展脚本的字段名），
//!   以及对象写法的单条规则 `{"type":"DOMAIN-SUFFIX","payload":"a.com","proxy":"PROXY"}`
//! * 纯文本：一行一条，允许带引号、尾逗号、`//` 注释（也就是扩展脚本里那个数组的正文）
//!
//! 统一做法是"把所有字符串叶子捞出来，逐条按 mihomo 的规则格式认"，认不出来的
//! **如实报告**而不是悄悄丢掉 —— 导入进去的东西会直接决定流量走哪。

use super::config::{NetworkProtocol, RuleAction, RuleMatcher, TrafficRule};
use serde_json::Value;

/// 本程序没有对应匹配类型的规则：导入时点名跳过，不假装成功。
/// 帮助弹框里"会跳过的类型"那一行也读这份清单，所以 match 分支必须用它。
pub const UNSUPPORTED_KINDS: &[&str] = &[
    "IP-CIDR6",
    "SRC-IP-CIDR",
    "PROCESS-CNAME",
    "DSCP",
    "INBOUND-TYPE",
    "MODE",
    "SCRIPT",
];

/// 一次导入的结果。`rules` 之外全是"没进来的部分"，界面上要逐条说清。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleImport {
    pub rules: Vec<TrafficRule>,
    /// 本程序没有这种匹配类型，整条跳过（`(类型名, 条数)`，按类型名排序）。
    pub unsupported: Vec<(String, usize)>,
    /// 捞到了字符串但完全不像规则。
    pub unparsed: Vec<String>,
    /// 目标写的是自定义代理组名（不是 DIRECT/PROXY/REJECT），被落到本程序代理组的条数。
    pub remapped_groups: usize,
    /// 原文里带 `no-resolve` 的条数 —— 本程序按匹配类型自己决定，不逐条存。
    pub no_resolve: usize,
}

impl RuleImport {
    /// 界面上那行汇总。条数为 0 的部分不写，别拿一串"0"糊脸。
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("可导入 {} 条", self.rules.len())];
        if self.remapped_groups > 0 {
            parts.push(format!(
                "{} 条目标组名落到本程序代理组",
                self.remapped_groups
            ));
        }
        if !self.unsupported.is_empty() {
            let listed = self
                .unsupported
                .iter()
                .map(|(kind, count)| format!("{kind}×{count}"))
                .collect::<Vec<_>>()
                .join("、");
            parts.push(format!("不支持 {listed}"));
        }
        if !self.unparsed.is_empty() {
            parts.push(format!("认不出 {} 条", self.unparsed.len()));
        }
        if self.no_resolve > 0 {
            parts.push(format!(
                "{} 条的 no-resolve 由本程序按类型决定",
                self.no_resolve
            ));
        }
        parts.join(" · ")
    }
}

/// 导出的形状：mihomo 规则字符串数组，缩进两格，方便直接贴回别的程序。
pub fn export_json(rules: &[TrafficRule]) -> Result<String, String> {
    let lines = export_lines(rules)?;
    serde_json::to_string_pretty(&lines).map_err(|error| error.to_string())
}

pub fn export_lines(rules: &[TrafficRule]) -> Result<Vec<String>, String> {
    rules
        .iter()
        .map(TrafficRule::to_mihomo_rule)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// 主入口。`text` 可以是 JSON，也可以是纯文本；两者都按"捞字符串 → 逐条认"处理。
pub fn parse_rules_text(text: &str) -> RuleImport {
    let mut import = RuleImport::default();
    for candidate in candidate_strings(text) {
        let line = clean_line(&candidate);
        if line.is_empty() {
            continue;
        }
        match parse_rule_line(&line) {
            Parsed::Rule(rule, flags) => {
                import.rules.push(rule);
                if flags.remapped {
                    import.remapped_groups += 1;
                }
                if flags.no_resolve {
                    import.no_resolve += 1;
                }
            }
            Parsed::Unsupported(kind) => bump_unsupported(&mut import.unsupported, kind),
            Parsed::Ignored => {}
            Parsed::Unparsed => import.unparsed.push(line),
        }
    }
    import.unsupported.sort();
    import
}

/// 解析单行规则字符串，成功返回 TrafficRule，无法解析或不受支持返回 None。
pub fn parse_single_rule(raw: &str) -> Option<TrafficRule> {
    let line = clean_line(raw);
    if line.is_empty() {
        return None;
    }
    match parse_rule_line(&line) {
        Parsed::Rule(rule, _) => Some(rule),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Flags {
    remapped: bool,
    no_resolve: bool,
}

/// 帮助弹框里的一段示范。`body` 必须**整段能导入**：下面的测试会逐段喂给
/// `parse_rules_text`，条数对不上或冒出"认不出"就不许写进界面 —— 界面上的例子
/// 就是他照着抄的规格，抄了导入失败比没例子更糟。
pub struct ImportExample {
    pub title: &'static str,
    pub note: &'static str,
    pub body: &'static str,
    pub expected_rules: usize,
    pub expected_unsupported: usize,
}

/// 支持的匹配类型（帮助弹框里逐行示范，也是解析器认账的全部类型）。
pub const SUPPORTED_KINDS: &[&str] = &[
    "DOMAIN",
    "DOMAIN-SUFFIX",
    "DOMAIN-KEYWORD",
    "GEOSITE",
    "GEOIP",
    "IP-CIDR",
    "DST-PORT",
    "PROCESS-NAME",
    "RULE-SET",
    "NETWORK",
    "QUIC",
];

pub fn import_examples() -> Vec<ImportExample> {
    vec![
        ImportExample {
            title: "一行一条（Clash 配置里 rules: 的正文）",
            note: "引号、单引号、尾逗号、`//` 与 `#` 注释都照吃，整段从脚本里复制即可。",
            body: "// ---- 本地 / 私有网络 ----\n\
                   \"IP-CIDR,172.16.0.0/12,DIRECT,no-resolve\",\n\
                   'DOMAIN,localhost,DIRECT',\n\
                   \"DST-PORT,22,DIRECT\" // SSH 直连\n",
            expected_rules: 3,
            expected_unsupported: 0,
        },
        ImportExample {
            title: "JSON 数组",
            note: "本程序「导出 JSON」产出的就是这个形状，导回去无损。",
            body: "[\n  \"DOMAIN-SUFFIX,example.com,PROXY\",\n  \
                   \"GEOSITE,cn,DIRECT\",\n  \
                   \"IP-CIDR,10.0.0.0/8,DIRECT,no-resolve\"\n]\n",
            expected_rules: 3,
            expected_unsupported: 0,
        },
        ImportExample {
            title: "JSON 对象（rules / prependRules 字段）",
            note: "Clash Verge 扩展脚本的字段名也认；目标写自定义组名的会落到本程序的代理组并点名。",
            body: "{\n  \"prependRules\": [\n    \
                   \"DOMAIN-SUFFIX,private.example,自定义组A\",\n    \
                   \"GEOSITE,category-ads-all,REJECT\"\n  ]\n}\n",
            expected_rules: 2,
            expected_unsupported: 0,
        },
        ImportExample {
            title: "对象写法的单条规则",
            note: "目标字段写 proxy、target 或 policy 都行。",
            body: "{\n  \"rules\": [\n    \
                   { \"type\": \"DOMAIN-SUFFIX\", \"payload\": \"a.com\", \"proxy\": \"PROXY\" },\n    \
                   { \"type\": \"DST-PORT\", \"payload\": \"443\", \"target\": \"DIRECT\" }\n  ]\n}\n",
            expected_rules: 2,
            expected_unsupported: 0,
        },
        ImportExample {
            title: "支持的匹配类型",
            note: "GEOIP 的 no-resolve 由本程序自动加；QUIC 简写和 AND 展开写法都收；DST-PORT 也可写成 DEST-PORT。",
            body: supported_kinds_example_body(),
            // 13 行：NETWORK 有 tcp/udp 两条，QUIC 的简写与 AND 展开写法各一条，DOMAIN-KEYWORD 一条。
            expected_rules: 13,
            expected_unsupported: 0,
        },
        ImportExample {
            title: "会点名跳过的类型",
            note: "本程序没有这些匹配类型，导入时逐条报数、绝不静默丢；MATCH 兜底行由本程序固定生成。",
            body: skipped_kinds_example_body(),
            expected_rules: 0,
            expected_unsupported: UNSUPPORTED_KINDS.len() + 1,
        },
    ]
}

fn supported_kinds_example_body() -> &'static str {
    // 手写而非拼接：例子内文要能直接读，且由测试保证与 SUPPORTED_KINDS 同步。
    "DOMAIN,example.com,PROXY\n\
     DOMAIN-SUFFIX,example.com,PROXY\n\
     DOMAIN-KEYWORD,openai,PROXY\n\
     GEOSITE,cn,DIRECT\n\
     GEOIP,CN,DIRECT\n\
     IP-CIDR,10.0.0.0/8,DIRECT,no-resolve\n\
     DST-PORT,443,PROXY\n\
     PROCESS-NAME,chrome,DIRECT\n\
     RULE-SET,ads,REJECT\n\
     NETWORK,tcp,PROXY\n\
     NETWORK,udp,DIRECT\n\
     QUIC,PROXY\n\
     AND,((NETWORK,udp),(DST-PORT,443)),PROXY\n"
}

fn skipped_kinds_example_body() -> &'static str {
    "IP-CIDR6,fc00::/7,DIRECT\n\
     SRC-IP-CIDR,10.0.0.0/8,PROXY\n\
     PROCESS-CNAME,chrome,DIRECT\n\
     DSCP,46,PROXY\n\
     INBOUND-TYPE,tun,PROXY\n\
     MODE,rule,DIRECT\n\
     SCRIPT,a.js,PROXY\n\
     MATCH,PROXY\n"
}

enum Parsed {
    Rule(TrafficRule, Flags),
    Unsupported(String),
    /// 注释行、空行、纯字段名（JSON 的 key 混进来时）。
    Ignored,
    Unparsed,
}

/// 把 JSON 里的字符串叶子都捞出来；不是合法 JSON 就退回按行切。
fn candidate_strings(text: &str) -> Vec<String> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        let mut out = Vec::new();
        collect_strings(&value, &mut out);
        if !out.is_empty() {
            return out;
        }
    }
    text.lines().map(str::to_string).collect()
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_strings(item, out);
            }
        }
        Value::Object(map) => {
            // 对象写法的单条规则（`{type, payload, proxy}`）拼回成一行，别丢。
            if let Some(line) = rule_from_object(map) {
                out.push(line);
                return;
            }
            for item in map.values() {
                collect_strings(item, out);
            }
        }
        _ => {}
    }
}

fn rule_from_object(map: &serde_json::Map<String, Value>) -> Option<String> {
    let text = |key: &str| match map.get(key) {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    };
    let kind = text("type")?;
    let payload = text("payload")?;
    let target = text("proxy")
        .or_else(|| text("target"))
        .or_else(|| text("policy"))?;
    Some(format!("{kind},{payload},{target}"))
}

/// 去掉包裹的引号、尾逗号、YAML 列表项 `- ` 前缀、`//` / `#` 注释与首尾空白。
fn clean_line(raw: &str) -> String {
    let mut line = raw.trim();
    if line.starts_with("//") || line.starts_with('#') {
        return String::new();
    }
    // 注释也可能跟在规则后面：`"DOMAIN,x,PROXY", // 说明` 或 `# 说明`
    if let Some((head, _)) = line.split_once("//") {
        line = head.trim_end();
    }
    if let Some((head, _)) = line.split_once('#') {
        line = head.trim_end();
    }
    line = line.trim();
    // 兼容 YAML 列表项前缀 `- ` 或 `-`
    if let Some(rest) = line.strip_prefix('-') {
        line = rest.trim();
    }
    line = line.trim_end_matches(',').trim();
    if line.len() >= 2
        && ((line.starts_with('"') && line.ends_with('"'))
            || (line.starts_with('\'') && line.ends_with('\'')))
    {
        line = &line[1..line.len() - 1];
    }
    line.trim().to_string()
}

fn bump_unsupported(list: &mut Vec<(String, usize)>, kind: String) {
    match list.iter_mut().find(|(name, _)| *name == kind) {
        Some((_, count)) => *count += 1,
        None => list.push((kind, 1)),
    }
}

fn action_for_target(target: &str) -> (RuleAction, bool) {
    match target.to_ascii_uppercase().as_str() {
        "DIRECT" => (RuleAction::Direct, false),
        "REJECT" => (RuleAction::Reject, false),
        "PROXY" => (RuleAction::Proxy, false),
        // 自定义组名：本程序只有一个代理组出口，落到它。
        _ => (RuleAction::Proxy, true),
    }
}

fn parse_rule_line(line: &str) -> Parsed {
    let parts: Vec<&str> = line.split(',').map(str::trim).collect();
    // 连逗号都没有的字符串不可能是规则（整份配置 JSON 里的名字、地址都会走到这里），
    // 把它们算进"认不出"只会淹掉真正的可疑行 —— 有逗号但认不出的才报。
    if parts.len() < 2 || parts[0].is_empty() {
        return Parsed::Ignored;
    }
    let kind = parts[0].to_ascii_uppercase();
    if kind == "MATCH" || kind == "FINAL" {
        // 兜底行由本程序固定生成（未命中走代理），不接受导入。
        return Parsed::Unsupported(kind);
    }
    // `AND,((NETWORK,udp),(DST-PORT,443)),目标` —— 载荷里全是逗号，只能按最后一个逗号切目标。
    if kind == "AND" {
        let payload_and_target = line.split_once(',').map(|(_, rest)| rest);
        let Some((payload, target)) = payload_and_target.and_then(|rest| rest.rsplit_once(','))
        else {
            return Parsed::Unparsed;
        };
        if !is_quic_payload(payload) {
            return Parsed::Unsupported(kind);
        }
        let (action, remapped) = action_for_target(target.trim());
        return Parsed::Rule(
            TrafficRule {
                matcher: RuleMatcher::Quic,
                action,
            },
            Flags {
                remapped,
                no_resolve: false,
            },
        );
    }
    // QUIC 只有两段；其余都是 `类型,值,目标[,修饰]`。
    let target_index = if kind == "QUIC" { 1 } else { 2 };
    let Some(target) = parts.get(target_index).filter(|value| !value.is_empty()) else {
        return Parsed::Unparsed;
    };
    let payload = parts
        .get(1)
        .filter(|value| !value.is_empty())
        .unwrap_or(&"");
    let no_resolve = parts
        .iter()
        .skip(target_index + 1)
        .any(|part| part.eq_ignore_ascii_case("no-resolve"));
    let (action, remapped) = action_for_target(target);
    let matcher = match kind.as_str() {
        "DOMAIN" if !payload.is_empty() => RuleMatcher::Domain(payload.to_string()),
        "DOMAIN-SUFFIX" if !payload.is_empty() => RuleMatcher::DomainSuffix(payload.to_string()),
        "DOMAIN-KEYWORD" if !payload.is_empty() => {
            RuleMatcher::DomainKeyword(payload.to_string())
        }
        "GEOSITE" if !payload.is_empty() => RuleMatcher::GeoSite(payload.to_string()),
        "GEOIP" if !payload.is_empty() => RuleMatcher::GeoIp(payload.to_string()),
        "IP-CIDR" if !payload.is_empty() => RuleMatcher::IpCidr(payload.to_string()),
        "DST-PORT" | "DEST-PORT" if !payload.is_empty() => {
            RuleMatcher::DestinationPort(payload.to_string())
        }
        "PROCESS-NAME" if !payload.is_empty() => RuleMatcher::ProcessName(payload.to_string()),
        "RULE-SET" if !payload.is_empty() => {
            let lower = payload.to_ascii_lowercase();
            match lower.as_str() {
                "reject" | "reject-list" | "ad" | "ads" | "advertising" => {
                    RuleMatcher::GeoSite("category-ads-all".into())
                }
                "direct" | "direct-list" | "cn" | "china" => {
                    RuleMatcher::GeoSite("cn".into())
                }
                "proxy" | "proxy-list" => {
                    RuleMatcher::GeoSite("geolocation-!cn".into())
                }
                "gfw" | "gfwlist" => {
                    RuleMatcher::GeoSite("gfw".into())
                }
                "cncidr" => {
                    RuleMatcher::GeoIp("CN".into())
                }
                "telegramcidr" => {
                    RuleMatcher::GeoIp("telegram".into())
                }
                "lancidr" | "private" => {
                    RuleMatcher::GeoSite("private".into())
                }
                "google" | "youtube" | "telegram" | "netflix" | "openai" | "chatgpt"
                | "microsoft" | "github" | "twitter" | "facebook" | "apple"
                | "speedtest" | "steam" => {
                    RuleMatcher::GeoSite(lower)
                }
                _ => RuleMatcher::RuleSet(payload.to_string()),
            }
        }
        "NETWORK" => match payload.to_ascii_lowercase().as_str() {
            "tcp" => RuleMatcher::Network(NetworkProtocol::Tcp),
            "udp" => RuleMatcher::Network(NetworkProtocol::Udp),
            _ => return Parsed::Unparsed,
        },
        // 本程序把它渲染成 `AND,((NETWORK,udp),(DST-PORT,443)),目标`，简写 QUIC 也照收。
        "QUIC" => RuleMatcher::Quic,
        // 本程序没有这种匹配类型 → 点名跳过（清单见 UNSUPPORTED_KINDS）。
        other if UNSUPPORTED_KINDS.contains(&other) => return Parsed::Unsupported(kind),
        _ => return Parsed::Unparsed,
    };
    Parsed::Rule(
        TrafficRule { matcher, action },
        Flags {
            remapped,
            no_resolve,
        },
    )
}

fn is_quic_payload(payload: &str) -> bool {
    payload
        .replace(' ', "")
        .eq_ignore_ascii_case("((network,udp),(dst-port,443))")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(rules: &[TrafficRule]) -> Vec<String> {
        rules
            .iter()
            .map(|rule| rule.to_mihomo_rule().unwrap())
            .collect()
    }

    /// JSON 数组：最常见的导出形状。
    #[test]
    fn imports_a_json_array_of_rule_strings() {
        let text = r#"["DOMAIN-SUFFIX,example.com,PROXY","IP-CIDR,10.0.0.0/8,DIRECT,no-resolve"]"#;
        let import = parse_rules_text(text);
        assert_eq!(
            kinds(&import.rules),
            vec![
                "DOMAIN-SUFFIX,example.com,PROXY",
                "IP-CIDR,10.0.0.0/8,DIRECT"
            ]
        );
        assert_eq!(
            import.no_resolve, 1,
            "原文带 no-resolve 要报出来，但不逐条存"
        );
        assert!(import.unsupported.is_empty() && import.unparsed.is_empty());
    }

    /// Clash Verge 那种扩展脚本：字段名是 prependRules，值是自定义组名。
    #[test]
    fn imports_clash_verge_shape_and_remaps_group_names() {
        let text = r#"{
            "prependRules": [
                "DOMAIN-SUFFIX,private.example,自定义组A",
                "DST-PORT,22,DIRECT",
                "GEOSITE,category-ads-all,REJECT"
            ]
        }"#;
        let import = parse_rules_text(text);
        assert_eq!(import.rules.len(), 3);
        assert_eq!(import.remapped_groups, 1, "只有那个组名要算改道");
        assert_eq!(
            kinds(&import.rules),
            vec![
                "DOMAIN-SUFFIX,private.example,PROXY",
                "DST-PORT,22,DIRECT",
                "GEOSITE,category-ads-all,REJECT"
            ]
        );
    }

    /// 他截图里那种 JS 正文：引号 + 尾逗号 + `//` 分节注释，全部按纯文本吃下。
    #[test]
    fn imports_plain_text_with_quotes_and_comments() {
        let text = "  // ---- 本地 / 私有网络 ----\n\
                    \"IP-CIDR,172.16.0.0/12,DIRECT,no-resolve\",\n\
                    'DOMAIN,localhost,DIRECT',\n\
                    \n\
                    // SSH\n\
                    \"DST-PORT,22,DIRECT\",\n";
        let import = parse_rules_text(text);
        assert_eq!(
            kinds(&import.rules),
            vec![
                "IP-CIDR,172.16.0.0/12,DIRECT",
                "DOMAIN,localhost,DIRECT",
                "DST-PORT,22,DIRECT"
            ]
        );
        assert!(import.unparsed.is_empty(), "{:?}", import.unparsed);
    }

    /// YAML 列表项破折号前缀与 # 注释写法解析验证
    #[test]
    fn imports_yaml_list_lines_with_dash_and_hash_comments() {
        let text = "# 自定义规则列表\n\
                    - DOMAIN-SUFFIX,openai.com,PROXY # AI 服务\n\
                    - 'DOMAIN-KEYWORD,google,PROXY'\n\
                    - PROCESS-NAME,IDMan.exe,DIRECT\n\
                    - GEOSITE,category-ads-all,REJECT\n";
        let import = parse_rules_text(text);
        assert_eq!(
            kinds(&import.rules),
            vec![
                "DOMAIN-SUFFIX,openai.com,PROXY",
                "DOMAIN-KEYWORD,google,PROXY",
                "PROCESS-NAME,IDMan.exe,DIRECT",
                "GEOSITE,category-ads-all,REJECT"
            ]
        );
        assert!(import.unparsed.is_empty(), "{:?}", import.unparsed);
    }

    /// 不支持的类型必须点名，不能假装导成功 —— 域名关键词和 IPv6 在真实规则表里很常见。
    #[test]
    fn reports_unsupported_types_instead_of_dropping_them() {
        let text = "[\"SRC-IP-CIDR,10.0.0.0/8,PROXY\",\"IP-CIDR6,fc00::/7,DIRECT\",\"DOMAIN-SUFFIX,a.com,PROXY\"]";
        let import = parse_rules_text(text);
        assert_eq!(import.rules.len(), 1);
        assert_eq!(
            import.unsupported,
            vec![
                ("IP-CIDR6".to_string(), 1),
                ("SRC-IP-CIDR".to_string(), 1)
            ]
        );
        assert!(
            import.summary().contains("IP-CIDR6×1、SRC-IP-CIDR×1"),
            "{}",
            import.summary()
        );
        assert!(
            import.summary().contains("可导入 1 条"),
            "{}",
            import.summary()
        );
    }

    /// 有逗号但完全不像规则的进 `unparsed`；连逗号都没有的（整份配置里的名字、地址）
    /// 不当可疑处理，否则真可疑行会被淹掉。
    #[test]
    fn junk_is_counted_as_unparsed() {
        let import =
            parse_rules_text("[\"hello world\",\"\",\"DOMAIN-SUFFIX,,PROXY\",\"FOO,a,PROXY\"]");
        assert!(import.rules.is_empty(), "{:?}", import.rules);
        assert_eq!(import.unparsed, vec!["DOMAIN-SUFFIX,,PROXY", "FOO,a,PROXY"]);
    }

    /// QUIC 的两种写法都要落回同一个匹配类型；渲染永远是本程序那一种规范形式。
    #[test]
    fn quic_is_recognised_in_both_forms_and_renders_canonically() {
        for line in ["QUIC,PROXY", "AND,((NETWORK,udp),(DST-PORT,443)),PROXY"] {
            let import = parse_rules_text(&format!("[\"{line}\"]"));
            assert_eq!(import.rules.len(), 1, "{line}");
            assert_eq!(import.rules[0].matcher, RuleMatcher::Quic, "{line}");
            assert_eq!(
                import.rules[0].to_mihomo_rule().unwrap(),
                "AND,((NETWORK,udp),(DST-PORT,443)),PROXY",
                "{line} 渲染成规范形式"
            );
        }
    }

    /// 对象写法的单条规则也认（有些工具导出成 `{type,payload,proxy}`）。
    #[test]
    fn imports_object_form_rules() {
        let import = parse_rules_text(
            r#"{"rules":[{"type":"DOMAIN-SUFFIX","payload":"a.com","proxy":"PROXY"}]}"#,
        );
        assert_eq!(kinds(&import.rules), vec!["DOMAIN-SUFFIX,a.com,PROXY"]);
    }

    /// 导出去再导入回来必须一模一样 —— 这是"能不能拿本程序的导出喂给别人"的底。
    #[test]
    fn export_then_import_is_lossless() {
        let import = parse_rules_text(
            "[\"DOMAIN-SUFFIX,a.com,PROXY\",\"GEOIP,CN,DIRECT\",\"DST-PORT,22,DIRECT\",\"PROCESS-NAME,steam,DIRECT\"]",
        );
        let json = export_json(&import.rules).expect("导出");
        let again = parse_rules_text(&json);
        assert_eq!(again.rules, import.rules, "导出→导入必须等价");
        assert!(again.unparsed.is_empty() && again.unsupported.is_empty());
        // GEOIP 的 no-resolve 是渲染时加的，导入端不该把它当第二条差异。
        assert!(json.contains("GEOIP,CN,DIRECT,no-resolve"), "{json}");
    }

    #[test]
    fn summary_omits_empty_buckets() {
        let import = parse_rules_text("[\"DOMAIN,x.com,PROXY\"]");
        assert_eq!(import.summary(), "可导入 1 条", "{}", import.summary());
    }

    /// 帮助弹框里的每个示范都必须**当场可解析**，且条数与标题里说的一致 ——
    /// 否则用户照着抄一段"示范"进去，得到的却是空导入。
    #[test]
    fn every_help_example_is_machine_checkable() {
        for example in import_examples() {
            let import = parse_rules_text(example.body);
            let skipped: usize = import.unsupported.iter().map(|(_, count)| count).sum();
            assert_eq!(
                import.rules.len(),
                example.expected_rules,
                "示范「{}」可导入条数对不上：{}",
                example.title,
                import.summary()
            );
            assert_eq!(
                skipped,
                example.expected_unsupported,
                "示范「{}」跳过条数对不上：{}",
                example.title,
                import.summary()
            );
            assert!(
                import.unparsed.is_empty(),
                "示范「{}」里有认不出的行：{:?}",
                example.title,
                import.unparsed
            );
            // 「复制这段」按钮塞进导入框后必须立刻出预览，且预览就是这段示范本身。
            let round = export_json(&import.rules).expect("示范解析后应可导出");
            assert_eq!(
                parse_rules_text(&round).rules,
                import.rules,
                "示范「{}」导出→导入不等价",
                example.title
            );
        }
    }

    /// 类型清单只有一份真相：SUPPORTED/UNSUPPORTED 改了，示范文本必须跟着改。
    #[test]
    fn the_kind_lists_and_the_examples_stay_in_sync() {
        let supported_body = supported_kinds_example_body().to_ascii_uppercase();
        for kind in SUPPORTED_KINDS {
            assert!(
                supported_body.contains(format!("{kind},").as_str()),
                "{kind} 声称支持却没写进示范"
            );
        }
        let skipped_body = skipped_kinds_example_body().to_ascii_uppercase();
        for kind in UNSUPPORTED_KINDS {
            assert!(
                skipped_body.contains(format!("{kind},").as_str()),
                "{kind} 属于跳过清单却没写进示范"
            );
            assert!(
                !supported_body.contains(format!("{kind},").as_str()),
                "{kind} 不能同时出现在支持示范里"
            );
        }
    }
}

#[cfg(test)]
mod verge_script_tests {
    use super::*;

    /// 用户 2026-10-14 截图里那份 Clash Verge 全局扩展脚本的正文（照原样抄，含组名、
    /// IPv6 与域名关键词）—— 这份能不能吃下、吃不下的怎么说，就是导入功能的验收线。
    const VERGE_BODY: &str = r#"
        // ---- 高优先级规则 ----
        // 本地 / 私有网络
        "DOMAIN-SUFFIX,private.example,自定义组A",
        "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
        "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve",
        "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
        // IPv6 私有地址保留直连规则
        "IP-CIDR6,fc00::/7,DIRECT,no-resolve",
        "IP-CIDR6,fe80::/10,DIRECT,no-resolve",
        // SSH
        "DST-PORT,22,DIRECT",
        // Runway
        "DOMAIN-SUFFIX,runwaym1.com,自定义组B",
        "DOMAIN-KEYWORD,runwaym1,自定义组B",
        "DOMAIN-SUFFIX,runway.com,自定义组B",
        // JavDB
        "DOMAIN-SUFFIX,javdb.com,自定义组A",
        "DOMAIN-SUFFIX,muse.ai,自定义组A",
"#;

    #[test]
    fn imports_a_real_clash_verge_script_body() {
        let import = parse_rules_text(VERGE_BODY);
        assert_eq!(
            import.rules.len(),
            10,
            "10 条能落进规则表：{}",
            import.summary()
        );
        assert_eq!(import.remapped_groups, 6, "六个自定义组名要点名");
        // 只数真正导入的那些：两条 IP-CIDR6 带着 no-resolve 但整条被跳过，不该计进来。
        assert_eq!(import.no_resolve, 3);
        assert_eq!(
            import.unsupported,
            vec![
                ("IP-CIDR6".to_string(), 2)
            ]
        );
        assert!(import.unparsed.is_empty(), "{:?}", import.unparsed);
        // 注释行、`// 分节` 与空行都不许被当成"认不出的规则"。
        assert_eq!(
            kinds(&import.rules)[..3],
            [
                "DOMAIN-SUFFIX,private.example,PROXY",
                "IP-CIDR,10.0.0.0/8,DIRECT",
                "IP-CIDR,172.16.0.0/12,DIRECT"
            ]
        );
        assert!(
            import
                .summary()
                .contains("不支持 IP-CIDR6×2"),
            "{}",
            import.summary()
        );
    }

    fn kinds(rules: &[TrafficRule]) -> Vec<String> {
        rules
            .iter()
            .map(|rule| rule.to_mihomo_rule().unwrap())
            .collect()
    }
}
