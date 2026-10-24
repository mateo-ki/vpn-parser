use crate::gui::theme;
use crate::models::{NodeGroupSummary, NodeSummary, GOOGLE_PROXY_TEST_KIND};
use crate::proxy::{
    DnsEnhancedMode, NetworkProtocol, ProxyMode, RoutingMode, RuleAction, RuleMatcher, TunStack,
};
use crate::speedtest::{SpeedTestPhase, SpeedTestProgress};
use eframe::egui::{self, Color32, RichText, Ui};

/// 全局 Ctrl+V / Ctrl+K 的"这次该不该让路" —— 只有焦点真的落在文本输入框里才让。
///
/// 为什么不能用 `memory().focused().is_none()`：egui 的 DragValue 点一下也会
/// `request_focus`（`egui-0.31.1/src/widgets/drag_value.rs:622`）。界面里"候选数 /
/// 延迟阈值 / 端口 / 并发数"全是 DragValue —— 用户拖完一个数字再按 Ctrl+V，
/// 就一声不响地什么都不做（2026-10-13 他连报两次"没识别"的根因）。
/// 所以让文本框在画的时候自己登记"焦点在我这"，读侧只认登记过的那个 id。
/// 新加文本框时记得调 `note_text_field`，漏了的后果是"在那个框里按 Ctrl+V 也会走识别"，
/// 是吵，不是哑 —— 挑这一侧是因为另一侧用户完全看不出来。
fn text_field_slot() -> egui::Id {
    egui::Id::new("vpn-parser:text-field-focus")
}

/// 每个文本输入框画完之后调一次；只有它真的有焦点时才写。
pub fn note_text_field(ui: &Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(text_field_slot(), response.id));
    }
}

/// 焦点现在在不在文本输入框里。
pub fn focus_is_text_field(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<egui::Id>(text_field_slot()))
        .is_some_and(|known| ctx.memory(|m| m.focused()) == Some(known))
}

pub fn normalize_api_base(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_owned()
    } else {
        // 发布服务器是远端机器，裸主机名默认按 https 处理，避免节点凭据明文上网。
        format!("https://{trimmed}")
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec >= 1024 * 1024 {
        format!("{:.2} MB/s", bytes_per_sec as f64 / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024 {
        format!("{:.1} KB/s", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{bytes_per_sec} B/s")
    }
}


/// 把 RFC3339 时间压成表头/日志里放得下的 `HH:MM`。解析不了就原样截五个字符，
/// 好过显示成看不懂的错误。
/// 把库里那条时间戳写成界面上的 `HH:MM`。
///
/// 存的是 UTC（RFC3339，`Utc::now().to_rfc3339()`），**显示一律换算成本地时区**，
/// 也就是他要的北京时间；`switch_day` 同理，日头按本地日切。
pub fn clock_of(at_rfc3339: &str) -> String {
    if let Ok(at) = chrono::DateTime::parse_from_rfc3339(at_rfc3339) {
        return at.with_timezone(&chrono::Local).format("%H:%M").to_string();
    }
    // 认不出的格式（例如 SQLite 默认那种 "2026-10-14 19:24:05"）宁可退回"取时间那一段"，
    // 也不能像以前那样取前 5 个字符 —— 那会在时间列上画出"2026-"。
    let bytes = at_rfc3339.as_bytes();
    if bytes.len() >= 16 && (bytes[10] == b'T' || bytes[10] == b' ') {
        return at_rfc3339[11..16].to_string();
    }
    "--:--".to_string()
}

pub fn format_duration_ms(value: Option<u64>) -> String {
    value
        .map(|ms| format!("{ms} ms"))
        .unwrap_or_else(|| "-".into())
}


/// 量着把一行文字截进 `max_width`：放得下原样返回（只量一次），放不下才二分找"接上
/// 省略号还塞得进"的最长前缀。逐字回退会让一条 300 字符的值每帧排 300 次版，二分是 9 次。
///
/// 为什么不靠 egui 自己截：`allocate_new_ui` 只改 `max_rect` 不改裁剪框，光把文字塞进
/// 子 ui 是裁不住的（关于页探针量到那条值的裁剪右界是整屏 1330，卡片右界只有 1290）；
/// 而就算 `set_clip_rect` 了，硬裁出来的也是半截字，不是省略号。
pub fn ellipsize(ui: &Ui, text: &str, max_width: f32, font: egui::FontId) -> String {
    const ELLIPSIS: char = '\u{2026}';
    let width = |candidate: &str| -> f32 {
        ui.ctx()
            .fonts(|fonts| {
                fonts.layout_no_wrap(candidate.to_string(), font.clone(), Color32::PLACEHOLDER)
            })
            .size()
            .x
    };
    if width(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let head: String = chars[..mid].iter().copied().collect();
        if width(&format!("{head}{ELLIPSIS}")) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let head: String = chars[..lo].iter().copied().collect();
    format!("{head}{ELLIPSIS}")
}

/// 清理订阅源常见的冗余前缀，让节点名称舒展通透、直奔主题
pub fn clean_node_display_name(name: &str) -> &str {
    let mut s = name.trim();
    const NOISY_PREFIXES: &[&str] = &[
        "地区随机 | ",
        "地区随机|",
        "Gslege优选 | ",
        "Gslege优选|",
        "全网优选 | ",
        "全网优选|",
        "自动优选 | ",
        "自动优选|",
        "优选节点 | ",
        "优选节点|",
        "VIP | ",
        "VIP|",
    ];
    for p in NOISY_PREFIXES {
        if let Some(rest) = s.strip_prefix(p) {
            s = rest.trim();
        }
    }
    s
}

/// 深度净化节点名称：去除乱码方块字符、单字母前缀、多余空格及杂乱符号
pub fn sanitize_node_name(name: &str) -> String {
    let raw = clean_node_display_name(name);
    // 过滤掉未解码的方块符号、乱码和控制字符
    let cleaned: String = raw
        .chars()
        .filter(|&c| c != '□' && c != '\u{FFFD}' && !c.is_control())
        .collect();

    let mut s = cleaned.trim();

    // 剔除常见的单字母前缀，如 "D 🇭🇰", "C 🇸🇬", "U · "
    if s.len() > 3 {
        let first_chars: Vec<char> = s.chars().take(4).collect();
        if first_chars.len() >= 2 && first_chars[1] == ' ' && first_chars[0].is_ascii_alphabetic() {
            s = &s[first_chars[0].len_utf8() + 1..];
        }
    }

    let s = s.trim();
    if s.is_empty() {
        raw.to_string()
    } else {
        s.to_string()
    }
}

pub fn latency_text(n: &NodeSummary) -> RichText {
    match (n.latency_ok, n.latency_test_kind.as_deref()) {
        (Some(true), Some(GOOGLE_PROXY_TEST_KIND)) => {
            let ms = n.latency_ms.unwrap_or(0);
            let color = if ms < 300 {
                theme::accent()
            } else if ms < 800 {
                theme::text_secondary()
            } else {
                theme::muted()
            };
            RichText::new(format!("{ms} ms")).color(color)
        }
        (Some(false), Some(GOOGLE_PROXY_TEST_KIND)) => {
            RichText::new("超时").color(theme::muted())
        }
        (Some(true), _) => {
            let ms = n.latency_ms.unwrap_or(0);
            RichText::new(format!("{ms} ms (TCP)")).color(theme::text_secondary())
        }
        (Some(false), _) => RichText::new("TCP 失败").color(theme::muted()),
        (None, _) => RichText::new("未测速").color(theme::muted()),
    }
}

pub fn routing_mode_description(mode: RoutingMode) -> &'static str {
    match mode {
        RoutingMode::Rule => "按本地自定义规则依次匹配",
        RoutingMode::ChinaDirect => "在线 Geo 规则 · 国内直连 · 其余代理",
        RoutingMode::GfwList => "在线 GFW 规则 · 命中代理 · 其余直连",
        RoutingMode::Global => "所有流量使用当前代理节点",
        RoutingMode::Direct => "所有流量绕过代理直接连接",
    }
}

pub fn routing_mode_detail(mode: RoutingMode) -> &'static str {
    match mode {
        RoutingMode::Rule => "保留并使用下方有序规则；没有命中的流量使用代理。",
        RoutingMode::ChinaDirect => {
            "自动使用广告、GeoSite CN 与 GeoIP CN 规则；在线数据由对应内核更新。"
        }
        RoutingMode::GfwList => "仅 GFW 规则集命中的站点使用代理，其他流量直接连接。",
        RoutingMode::Global => "忽略自定义规则，所有可代理流量都使用当前节点。",
        RoutingMode::Direct => "忽略自定义规则，代理内核保持运行但流量全部直连。",
    }
}

pub fn source_status_failed(status: &str) -> bool {
    let status = status.trim().to_ascii_lowercase();
    !status.is_empty()
        && status != "ok"
        && status != "not modified"
        && status != "content unchanged"
}


pub fn tun_stack_label(stack: TunStack) -> &'static str {
    match stack {
        TunStack::System => "System",
        TunStack::Gvisor => "gVisor",
        TunStack::Mixed => "Mixed",
    }
}

pub fn dns_mode_label(mode: DnsEnhancedMode) -> &'static str {
    match mode {
        DnsEnhancedMode::FakeIp => "Fake-IP",
        DnsEnhancedMode::RedirHost => "Redir-Host",
    }
}

pub fn rule_action_label(action: RuleAction) -> &'static str {
    match action {
        RuleAction::Direct => "直连",
        RuleAction::Proxy => "代理",
        RuleAction::Reject => "拒绝",
    }
}

pub fn rule_matcher_label(matcher: &RuleMatcher) -> String {
    match matcher {
        RuleMatcher::Domain(value) => format!("DOMAIN · {value}"),
        RuleMatcher::DomainSuffix(value) => format!("DOMAIN-SUFFIX · {value}"),
        RuleMatcher::DomainKeyword(value) => format!("DOMAIN-KEYWORD · {value}"),
        RuleMatcher::GeoSite(value) => format!("GEOSITE · {value}"),
        RuleMatcher::GeoIp(value) => format!("GEOIP · {value}"),
        RuleMatcher::IpCidr(value) => format!("IP-CIDR · {value}"),
        RuleMatcher::DestinationPort(value) => format!("DST-PORT · {value}"),
        RuleMatcher::ProcessName(value) => format!("PROCESS-NAME · {value}"),
        RuleMatcher::RuleSet(value) => format!("RULE-SET · {value}"),
        RuleMatcher::Network(NetworkProtocol::Tcp) => "NETWORK · tcp".into(),
        RuleMatcher::Network(NetworkProtocol::Udp) => "NETWORK · udp".into(),
        RuleMatcher::Quic => "QUIC · udp/443".into(),
    }
}

pub fn sort_label(v: &str) -> &'static str {
    match v {
        "latency" => "延迟",
        "download" => "下载速度",
        "name" => "名称",
        _ => "更新时间",
    }
}

pub fn format_speedtest_progress(progress: &SpeedTestProgress) -> String {
    let phase = match progress.phase {
        SpeedTestPhase::TcpPrefilter => "TCP预筛",
        SpeedTestPhase::RealProxy => "真实代理验证",
    };
    let percent = if progress.total == 0 {
        100.0
    } else {
        progress.done as f64 * 100.0 / progress.total as f64
    };
    let detail = match (progress.kernel, progress.batch_index, progress.batch_total) {
        (Some(kernel), Some(batch), Some(total)) => {
            format!(" · {} 批次 {batch}/{total}", kernel.display_name())
        }
        (Some(kernel), _, _) => format!(" · {}", kernel.display_name()),
        _ => String::new(),
    };
    format!(
        "{phase}{detail} {}/{}（{percent:.1}%） 成功 {} / 失败 {}，耗时 {:.1}s",
        progress.done,
        progress.total,
        progress.ok,
        progress.fail,
        progress.elapsed.as_secs_f64()
    )
}

pub fn nonempty_opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub fn node_group_key(group: &NodeGroupSummary) -> String {
    group
        .source_id
        .clone()
        .unwrap_or_else(|| crate::db::UNGROUPED_SOURCE_ID.into())
}

pub fn requires_tun_elevation(mode: ProxyMode, elevated: bool) -> bool {
    if cfg!(windows) {
        mode.has_tun() && !elevated
    } else {
        // Linux 下 GUI 始终以当前登录用户运行，严禁使用 sudo 启动 GUI（会导致 Wayland 鉴权失败与目录跑偏）。
        // TUN 网络权限由内核二进制文件通过 setcap 独立持有。
        false
    }
}
