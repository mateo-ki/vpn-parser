use crate::gui::theme;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2,
};

#[derive(Debug, PartialEq, Eq)]
pub enum InlineSpan<'a> {
    Text(&'a str),
    Bold(&'a str),
    Italic(&'a str),
    Code(&'a str),
    Link { label: &'a str, url: &'a str },
}

/// 解析单行文本内的行内 Markdown 样式
pub fn parse_inline_spans(text: &str) -> Vec<InlineSpan<'_>> {
    let mut spans = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        let find_candidate = |rem: &str, pat: &'static str| rem.find(pat).map(|idx| (idx, pat));

        let mut next_token: Option<(usize, &str)> = None;
        for pat in &["`", "**", "*", "[", "https://", "http://"] {
            if let Some((idx, p)) = find_candidate(remaining, pat) {
                // 若为单个 * 但位于 ** 中，跳过
                if p == "*" && remaining[idx..].starts_with("**") {
                    continue;
                }
                if next_token.is_none() || idx < next_token.unwrap().0 {
                    next_token = Some((idx, p));
                }
            }
        }

        let Some((idx, pat)) = next_token else {
            spans.push(InlineSpan::Text(remaining));
            break;
        };

        if idx > 0 {
            spans.push(InlineSpan::Text(&remaining[..idx]));
            remaining = &remaining[idx..];
        }

        match pat {
            "`" => {
                let after_first = &remaining[1..];
                if let Some(close_idx) = after_first.find('`') {
                    spans.push(InlineSpan::Code(&after_first[..close_idx]));
                    remaining = &after_first[close_idx + 1..];
                } else {
                    spans.push(InlineSpan::Text("`"));
                    remaining = after_first;
                }
            }
            "**" => {
                let after_first = &remaining[2..];
                if let Some(close_idx) = after_first.find("**") {
                    spans.push(InlineSpan::Bold(&after_first[..close_idx]));
                    remaining = &after_first[close_idx + 2..];
                } else {
                    spans.push(InlineSpan::Text("**"));
                    remaining = after_first;
                }
            }
            "*" => {
                let after_first = &remaining[1..];
                if let Some(close_idx) = after_first.find('*') {
                    if !after_first[close_idx..].starts_with("**") {
                        spans.push(InlineSpan::Italic(&after_first[..close_idx]));
                        remaining = &after_first[close_idx + 1..];
                        continue;
                    }
                }
                spans.push(InlineSpan::Text("*"));
                remaining = after_first;
            }
            "[" => {
                let after_bracket = &remaining[1..];
                if let Some(close_bracket) = after_bracket.find("](") {
                    let label = &after_bracket[..close_bracket];
                    let after_paren = &after_bracket[close_bracket + 2..];
                    if let Some(close_paren) = after_paren.find(')') {
                        let url = &after_paren[..close_paren];
                        spans.push(InlineSpan::Link { label, url });
                        remaining = &after_paren[close_paren + 1..];
                        continue;
                    }
                }
                spans.push(InlineSpan::Text("["));
                remaining = after_bracket;
            }
            "https://" | "http://" => {
                let end = remaining
                    .find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '>' | '，' | '。' | ','))
                    .unwrap_or(remaining.len());
                let url = &remaining[..end];
                spans.push(InlineSpan::Link { label: url, url });
                remaining = &remaining[end..];
            }
            _ => {
                spans.push(InlineSpan::Text(&remaining[..pat.len()]));
                remaining = &remaining[pat.len()..];
            }
        }
    }

    spans
}

/// 解析常规 Conventional Commit 前缀
fn parse_list_item_prefix(text: &str) -> (Option<(&'static str, Color32, Color32)>, Option<String>, &str) {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("[x]") {
        return (Some(("已完成", theme::success(), theme::success_bg())), None, rest.trim_start());
    }
    if let Some(rest) = trimmed.strip_prefix("[ ]") {
        return (Some(("待办", theme::muted(), theme::sidebar())), None, rest.trim_start());
    }

    let prefixes: &[(&str, &'static str, fn() -> (Color32, Color32))] = &[
        ("feat", "新增", || (theme::success(), theme::success_bg())),
        ("feature", "新增", || (theme::success(), theme::success_bg())),
        ("fix", "修复", || (theme::warning(), theme::warning_bg())),
        ("bug", "修复", || (theme::warning(), theme::warning_bg())),
        ("perf", "优化", || (theme::accent(), theme::accent_bg())),
        ("refactor", "重构", || (Color32::from_rgb(168, 85, 247), Color32::from_rgb(168, 85, 247).gamma_multiply(0.18))),
        ("chore", "维护", || (theme::text_secondary(), theme::panel_hover())),
        ("docs", "文档", || (Color32::from_rgb(2, 132, 199), Color32::from_rgb(2, 132, 199).gamma_multiply(0.18))),
        ("style", "样式", || (Color32::from_rgb(236, 72, 153), Color32::from_rgb(236, 72, 153).gamma_multiply(0.18))),
        ("test", "测试", || (Color32::from_rgb(16, 185, 129), Color32::from_rgb(16, 185, 129).gamma_multiply(0.18))),
        ("breaking", "破坏性", || (theme::danger(), theme::danger_bg())),
    ];

    for &(keyword, label, color_fn) in prefixes {
        if trimmed.starts_with(keyword) {
            let after_kw = &trimmed[keyword.len()..];
            if after_kw.starts_with('(') {
                if let Some(close_idx) = after_kw.find(')') {
                    let scope = &after_kw[1..close_idx];
                    let after_paren = &after_kw[close_idx + 1..];
                    if after_paren.starts_with(':') {
                        let (fg, bg) = color_fn();
                        let rest = after_paren[1..].trim_start();
                        return (Some((label, fg, bg)), Some(scope.trim().to_string()), rest);
                    }
                }
            } else if after_kw.starts_with(':') {
                let (fg, bg) = color_fn();
                let rest = after_kw[1..].trim_start();
                return (Some((label, fg, bg)), None, rest);
            }
        }
    }

    (None, None, trimmed)
}

/// 解析 GitHub Release 常见的尾部 "by @author in https://..."
fn parse_github_release_suffix(text: &str) -> (&str, Option<String>, Option<(String, String)>) {
    let mut author: Option<String> = None;
    let mut pr_link: Option<(String, String)> = None;
    let mut clean_text = text;

    if let Some(in_idx) = clean_text.rfind(" in https://") {
        let url = clean_text[in_idx + 4..].trim();
        if let Some(pull_idx) = url.rfind("/pull/") {
            let pr_num = &url[pull_idx + 6..].trim_end_matches('/');
            if pr_num.chars().all(|c| c.is_ascii_digit()) {
                pr_link = Some((format!("PR #{pr_num}"), url.to_string()));
            } else {
                pr_link = Some(("查看 PR".to_string(), url.to_string()));
            }
        } else {
            pr_link = Some(("链接".to_string(), url.to_string()));
        }
        clean_text = clean_text[..in_idx].trim_end();
    }

    if let Some(by_idx) = clean_text.rfind(" by @") {
        let username = clean_text[by_idx + 5..].trim();
        if !username.contains(' ') {
            author = Some(format!("@{username}"));
            clean_text = clean_text[..by_idx].trim_end();
        }
    }

    (clean_text, author, pr_link)
}

/// 匹配标题关键词以附加优雅图标
fn match_heading_icon(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if lower.contains("what's changed") || lower.contains("变更") || lower.contains("更新日志") || lower.contains("changelog") {
        "🚀"
    } else if lower.contains("feat") || lower.contains("特性") || lower.contains("新功能") || lower.contains("亮点") {
        "✨"
    } else if lower.contains("fix") || lower.contains("修复") || lower.contains("bug") {
        "🐛"
    } else if lower.contains("perf") || lower.contains("性能") || lower.contains("优化") {
        "⚡"
    } else if lower.contains("refactor") || lower.contains("重构") {
        "♻️"
    } else if lower.contains("breaking") || lower.contains("破坏") || lower.contains("警告") {
        "⚠️"
    } else if lower.contains("upgrade") || lower.contains("升级") || lower.contains("指南") || lower.contains("说明") {
        "💡"
    } else if lower.contains("link") || lower.contains("链接") {
        "🔗"
    } else {
        "📌"
    }
}

/// 渲染行内富文本
fn render_inline_markdown(ui: &mut Ui, text: &str, font_size: f32, default_color: Color32, italic: bool) {
    let wrap_w = if ui.available_width().is_finite() && ui.available_width() > 50.0 {
        ui.available_width()
    } else {
        320.0
    };
    let spans = parse_inline_spans(text);
    let mut job = egui::text::LayoutJob::default();
    job.break_on_newline = true;
    job.wrap.max_width = wrap_w;
    let mut first_link_url = None;

    for span in spans {
        match span {
            InlineSpan::Text(s) => {
                job.append(s, 0.0, egui::text::TextFormat {
                    font_id: FontId::proportional(font_size),
                    color: default_color,
                    italics: italic,
                    ..Default::default()
                });
            }
            InlineSpan::Bold(s) => {
                job.append(s, 0.0, egui::text::TextFormat {
                    font_id: FontId::proportional(font_size),
                    color: theme::text_primary(),
                    ..Default::default()
                });
            }
            InlineSpan::Italic(s) => {
                job.append(s, 0.0, egui::text::TextFormat {
                    font_id: FontId::proportional(font_size),
                    color: default_color,
                    italics: true,
                    ..Default::default()
                });
            }
            InlineSpan::Code(s) => {
                job.append(s, 0.0, egui::text::TextFormat {
                    font_id: FontId::monospace(font_size - 1.0),
                    color: theme::accent_strong(),
                    background: theme::panel_hover(),
                    ..Default::default()
                });
            }
            InlineSpan::Link { label, url } => {
                if first_link_url.is_none() {
                    first_link_url = Some(url.to_string());
                }
                job.append(label, 0.0, egui::text::TextFormat {
                    font_id: FontId::proportional(font_size),
                    color: theme::accent(),
                    underline: Stroke::new(1.0, theme::accent()),
                    ..Default::default()
                });
            }
        }
    }

    if let Some(url) = first_link_url {
        let resp = ui.add(egui::Label::new(job).sense(Sense::click()));
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("在系统默认浏览器中打开: {url}"))
            .clicked()
        {
            let _ = crate::platform::open_url_in_browser(&url);
            ui.ctx().open_url(egui::OpenUrl::new_tab(&url));
        }
    } else {
        ui.label(job);
    }
}

/// 渲染单条列表项，确保文本与微标全部融入单个 LayoutJob 并在限定宽度内自动换行，绝不横向撑爆容器
fn render_list_item(ui: &mut Ui, line: &str, is_unordered: bool) {
    let trimmed = line.trim();
    let indent = (line.len() - line.trim_start().len()) / 2;
    let raw_content = if is_unordered {
        &trimmed[2..]
    } else {
        let dot_idx = trimmed.find('.').unwrap_or(0);
        &trimmed[(dot_idx + 2).min(trimmed.len())..]
    };

    let (clean_text, author, pr_link) = parse_github_release_suffix(raw_content);
    let (badge, scope, text) = parse_list_item_prefix(clean_text);

    let wrap_w = if ui.available_width().is_finite() && ui.available_width() > 50.0 {
        ui.available_width()
    } else {
        320.0
    };

    let mut job = egui::text::LayoutJob::default();
    job.break_on_newline = true;
    job.wrap.max_width = wrap_w;

    // 1. 缩进处理
    if indent > 0 {
        let spaces = "  ".repeat(indent.min(4));
        job.append(&spaces, 0.0, egui::text::TextFormat::default());
    }

    // 2. 列表圆点
    job.append("● ", 0.0, egui::text::TextFormat {
        font_id: FontId::proportional(8.0),
        color: theme::accent(),
        valign: egui::Align::Center,
        ..Default::default()
    });

    // 3. 常规提交前缀微标 (如 [修复], [新增])
    if let Some((badge_label, fg, bg)) = badge {
        job.append(&format!("[{badge_label}] "), 0.0, egui::text::TextFormat {
            font_id: FontId::monospace(10.5),
            color: fg,
            background: bg,
            valign: egui::Align::Center,
            ..Default::default()
        });
    }

    // 4. 作用域微标 (如 (gui) 或 (proxy))
    if let Some(scope_text) = scope {
        job.append(&format!("({scope_text}) "), 0.0, egui::text::TextFormat {
            font_id: FontId::proportional(11.0),
            color: theme::muted(),
            valign: egui::Align::Center,
            ..Default::default()
        });
    }

    // 5. 列表项正文 (解析加粗、代码块、链接)
    let spans = parse_inline_spans(text);
    for span in spans {
        match span {
            InlineSpan::Text(s) => {
                job.append(s, 0.0, egui::text::TextFormat {
                    font_id: FontId::proportional(12.0),
                    color: theme::text_primary(),
                    ..Default::default()
                });
            }
            InlineSpan::Bold(s) => {
                job.append(s, 0.0, egui::text::TextFormat {
                    font_id: FontId::proportional(12.0),
                    color: theme::text_primary(),
                    ..Default::default()
                });
            }
            InlineSpan::Italic(s) => {
                job.append(s, 0.0, egui::text::TextFormat {
                    font_id: FontId::proportional(12.0),
                    color: theme::text_secondary(),
                    italics: true,
                    ..Default::default()
                });
            }
            InlineSpan::Code(s) => {
                job.append(s, 0.0, egui::text::TextFormat {
                    font_id: FontId::monospace(11.0),
                    color: theme::accent_strong(),
                    background: theme::panel_hover(),
                    ..Default::default()
                });
            }
            InlineSpan::Link { label, .. } => {
                job.append(label, 0.0, egui::text::TextFormat {
                    font_id: FontId::proportional(12.0),
                    color: theme::accent(),
                    underline: Stroke::new(1.0, theme::accent()),
                    ..Default::default()
                });
            }
        }
    }

    // 6. 作者尾缀
    if let Some(author_text) = author {
        job.append(&format!(" {author_text}"), 0.0, egui::text::TextFormat {
            font_id: FontId::proportional(11.0),
            color: theme::muted(),
            ..Default::default()
        });
    }

    // 7. PR 链接尾缀
    if let Some((ref pr_label, _)) = pr_link {
        job.append(&format!(" {pr_label}"), 0.0, egui::text::TextFormat {
            font_id: FontId::proportional(11.0),
            color: theme::accent(),
            underline: Stroke::new(1.0, theme::accent()),
            ..Default::default()
        });
    }

    ui.add_space(2.0);
    if let Some((_, pr_url)) = pr_link {
        let resp = ui.add(egui::Label::new(job).sense(Sense::click()));
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("在系统默认浏览器中打开: {pr_url}"))
            .clicked()
        {
            let _ = crate::platform::open_url_in_browser(&pr_url);
            ui.ctx().open_url(egui::OpenUrl::new_tab(&pr_url));
        }
    } else {
        ui.label(job);
    }
    ui.add_space(2.0);
}

/// 渲染 Markdown 格式文本的专用美化组件
pub fn render_markdown(ui: &mut Ui, markdown: &str) {
    let mut in_code_block = false;
    let mut code_block_lang = String::new();
    let mut code_block_content = Vec::new();

    let lines: Vec<&str> = markdown.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // 1. 代码块处理
        if trimmed.starts_with("```") {
            if in_code_block {
                // 代码块结束
                in_code_block = false;
                let full_code = code_block_content.join("\n");
                code_block_content.clear();

                ui.add_space(4.0);
                Frame::NONE
                    .fill(if theme::is_light() { Color32::from_rgb(245, 247, 250) } else { Color32::from_rgb(20, 22, 26) })
                    .stroke(Stroke::new(1.0, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        if !code_block_lang.is_empty() {
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(&code_block_lang).small().monospace().color(theme::muted()));
                                });
                            });
                            ui.add_space(2.0);
                        }
                        let mut job = egui::text::LayoutJob::default();
                        job.break_on_newline = true;
                        job.wrap.max_width = (ui.available_width() - 8.0).max(60.0);
                        job.append(&full_code, 0.0, egui::text::TextFormat {
                            font_id: FontId::monospace(12.0),
                            color: theme::text_primary(),
                            ..Default::default()
                        });
                        ui.label(job);
                    });
                ui.add_space(6.0);
            } else {
                // 代码块开始
                in_code_block = true;
                code_block_lang = trimmed.trim_start_matches('`').trim().to_string();
            }
            i += 1;
            continue;
        }

        if in_code_block {
            code_block_content.push(line);
            i += 1;
            continue;
        }

        // 2. 空行
        if trimmed.is_empty() {
            ui.add_space(3.0);
            i += 1;
            continue;
        }

        // 3. 分割线
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            ui.add_space(5.0);
            let w = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, theme::border_subtle());
            ui.add_space(5.0);
            i += 1;
            continue;
        }

        // 4. 标题解析
        if let Some(h1) = trimmed.strip_prefix("# ") {
            ui.add_space(6.0);
            ui.label(RichText::new(h1.trim()).size(16.0).strong().color(theme::accent_strong()));
            ui.add_space(3.0);
            let w = ui.available_width().min(240.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.5), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(1), theme::accent().gamma_multiply(0.4));
            ui.add_space(4.0);
            i += 1;
            continue;
        }
        if let Some(h2) = trimmed.strip_prefix("## ") {
            ui.add_space(6.0);
            let title = h2.trim();
            let icon = match_heading_icon(title);
            ui.label(RichText::new(format!("{icon} {title}")).size(14.0).strong().color(theme::text_primary()));
            ui.add_space(3.0);
            i += 1;
            continue;
        }
        if let Some(h3) = trimmed.strip_prefix("### ") {
            ui.add_space(4.0);
            let title = h3.trim();
            ui.label(RichText::new(title).size(13.0).strong().color(theme::text_primary()));
            ui.add_space(2.0);
            i += 1;
            continue;
        }
        if let Some(h4) = trimmed.strip_prefix("#### ") {
            ui.add_space(3.0);
            let title = h4.trim();
            ui.label(RichText::new(title).size(12.0).strong().color(theme::text_secondary()));
            ui.add_space(2.0);
            i += 1;
            continue;
        }

        // 5. 引用块
        if let Some(quote) = trimmed.strip_prefix('>') {
            let quote_text = quote.trim();
            ui.add_space(3.0);
            Frame::NONE
                .fill(theme::panel_hover())
                .stroke(Stroke::new(1.0, theme::border_subtle()))
                .corner_radius(CornerRadius::same(5))
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    render_inline_markdown(ui, quote_text, 12.0, theme::text_secondary(), true);
                });
            ui.add_space(3.0);
            i += 1;
            continue;
        }

        // 6. 列表项解析 (* / - / + / 数字序号)
        let is_unordered_bullet = trimmed.starts_with("* ") || trimmed.starts_with("- ") || trimmed.starts_with("+ ");
        let is_numbered = trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            && trimmed.find('.').map(|idx| idx < 4 && trimmed[idx..].starts_with(". ")).unwrap_or(false);

        if is_unordered_bullet || is_numbered {
            render_list_item(ui, line, is_unordered_bullet);
            i += 1;
            continue;
        }

        // 7. 普通段落
        ui.add_space(1.5);
        render_inline_markdown(ui, trimmed, 12.0, theme::text_primary(), false);
        ui.add_space(2.5);
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_inline_spans() {
        let text = "优化 **性能** 与 `Cargo.lock`，详情见 [说明](https://example.com)！";
        let spans = parse_inline_spans(text);
        assert_eq!(
            spans,
            vec![
                InlineSpan::Text("优化 "),
                InlineSpan::Bold("性能"),
                InlineSpan::Text(" 与 "),
                InlineSpan::Code("Cargo.lock"),
                InlineSpan::Text("，详情见 "),
                InlineSpan::Link { label: "说明", url: "https://example.com" },
                InlineSpan::Text("！"),
            ]
        );
    }

    #[test]
    fn test_parse_list_item_prefix() {
        let (badge, scope, rest) = parse_list_item_prefix("fix(gui): 默认首选 Glow 渲染器");
        assert_eq!(badge.map(|b| b.0), Some("修复"));
        assert_eq!(scope, Some("gui".to_string()));
        assert_eq!(rest, "默认首选 Glow 渲染器");

        let (badge, scope, rest) = parse_list_item_prefix("feat: 支持 IPv6 双栈");
        assert_eq!(badge.map(|b| b.0), Some("新增"));
        assert_eq!(scope, None);
        assert_eq!(rest, "支持 IPv6 双栈");

        let (badge, scope, rest) = parse_list_item_prefix("[x] 已完成单元测试");
        assert_eq!(badge.map(|b| b.0), Some("已完成"));
        assert_eq!(scope, None);
        assert_eq!(rest, "已完成单元测试");
    }

    #[test]
    fn test_parse_github_release_suffix() {
        let text = "修复渲染频闪 by @mateo-ki in https://github.com/mateo-ki/vproxy/pull/123";
        let (clean, author, pr_link) = parse_github_release_suffix(text);
        assert_eq!(clean, "修复渲染频闪");
        assert_eq!(author, Some("@mateo-ki".to_string()));
        assert_eq!(pr_link, Some(("PR #123".to_string(), "https://github.com/mateo-ki/vproxy/pull/123".to_string())));
    }

    #[test]
    fn test_render_markdown_smoke() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let md = "# 标题\n\n> 引用块\n\n* feat(gui): 修复超长中文文本换行测试 by @mateo-ki in https://github.com/mateo-ki/vproxy/pull/1\n- 普通列表项包含 `代码` 与 **加粗**\n\n```rust\nfn main() {}\n```\n";
            render_markdown(ui, md);
        });
        output.textures_delta.clear();
    }
}
