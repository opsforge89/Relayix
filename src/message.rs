//! Delivery presentation is independent from the event title used in history.
use crate::*;

pub const TITLE_SQL:&str="COALESCE(CASE WHEN jsonb_typeof(COALESCE(e.rendered_payload,e.payload)->'title')='string' AND COALESCE(e.rendered_payload,e.payload)->>'title' ~ '[^[:space:]]' THEN COALESCE(e.rendered_payload,e.payload)->>'title' END,CASE WHEN jsonb_typeof(COALESCE(e.rendered_payload,e.payload)->'alertname')='string' AND COALESCE(e.rendered_payload,e.payload)->>'alertname' ~ '[^[:space:]]' THEN COALESCE(e.rendered_payload,e.payload)->>'alertname' END,'')";

pub struct Rendered {
    pub text: String,
    pub html: bool,
    pub title: String,
    pub body: String,
}
/// Missing/blank titles never inherit the history's "Notification" fallback.
pub fn event_title(payload: &Value) -> &str {
    payload
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            payload
                .get("alertname")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or("")
}
pub fn event_message(payload: &Value) -> &str {
    payload
        .get("message")
        .or_else(|| payload.get("body"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn escape_markdown(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '>' => out.push_str("&gt;"),
            '<' => out.push_str("&lt;"),
            c if "\\`*_{}[]()#+-.!|>~".contains(c) => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}
#[derive(Debug)]
enum Token {
    Text(String),
    Mark,
}
fn spans(text: &str) -> Vec<(String, bool)> {
    let mut tokens = Vec::new();
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'\n') {
            chars.next();
            plain.push('\n');
        } else if c == '\\' && matches!(chars.peek(), Some('*' | '\\')) {
            plain.push(chars.next().unwrap());
        } else if c == '*' && chars.peek() == Some(&'*') {
            chars.next();
            if !plain.is_empty() {
                tokens.push(Token::Text(std::mem::take(&mut plain)));
            }
            tokens.push(Token::Mark);
        } else {
            plain.push(c);
        }
    }
    if !plain.is_empty() {
        tokens.push(Token::Text(plain));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Text(text) => {
                out.push((text.clone(), false));
                i += 1;
            }
            Token::Mark => {
                let next = (i + 1..tokens.len()).find(|&j| matches!(tokens[j], Token::Mark));
                if let Some(j) = next.filter(|&j| j > i + 1) {
                    let text = tokens[i + 1..j]
                        .iter()
                        .filter_map(|t| {
                            if let Token::Text(s) = t {
                                Some(s.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<String>();
                    if !text.trim().is_empty() {
                        out.push((text, true));
                        i = j + 1;
                        continue;
                    }
                }
                out.push(("**".to_owned(), false));
                i += 1;
            }
        }
    }
    out
}
fn escaped(kind: &str, text: &str) -> String {
    if kind == "telegram" {
        escape_html(text)
    } else {
        escape_markdown(text)
    }
}
fn link_at(text: &str) -> Option<(usize, &str, String)> {
    let close = text.find("](")?;
    let label = &text[1..close];
    if label.trim().is_empty() {
        return None;
    }
    let start = close + 2;
    let mut depth = 1;
    for (offset, c) in text[start..].char_indices() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
        }
        if depth == 0 {
            let raw = &text[start..start + offset];
            if raw.chars().any(|c| c.is_whitespace() || c == '\\') {
                return None;
            }
            let url = url::Url::parse(raw).ok()?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return None;
            }
            return Some((start + offset + 1, label, url.to_string()));
        }
    }
    None
}
fn links(kind: &str, text: &str) -> String {
    let mut out = String::new();
    let mut literal = String::new();
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        if rest.starts_with("\\[") || rest.starts_with("\\]") {
            literal.push(rest.as_bytes()[1] as char);
            index += 2;
            continue;
        }
        if rest.starts_with('[') {
            if let Some((length, label, url)) = link_at(rest) {
                out.push_str(&escaped(kind, &literal));
                literal.clear();
                if kind == "telegram" {
                    out.push_str(&format!(
                        "<a href=\"{}\">{}</a>",
                        escape_html(&url),
                        escape_html(label)
                    ));
                } else {
                    out.push_str(&format!(
                        "[{}]({})",
                        escape_markdown(label),
                        url.replace('(', "%28").replace(')', "%29")
                    ));
                }
                index += length;
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        literal.push(c);
        index += c.len_utf8();
    }
    out.push_str(&escaped(kind, &literal));
    out
}
fn formatted(kind: &str, text: &str, bold_title: bool) -> String {
    let parts = if bold_title {
        vec![(text.to_owned(), true)]
    } else {
        spans(text)
    };
    let mut out = String::new();
    for (text, bold) in parts {
        let escaped = if bold_title {
            escaped(kind, &text)
        } else {
            links(kind, &text)
        };
        if bold {
            out.push_str(if kind == "telegram" { "<b>" } else { "**" });
        }
        out.push_str(&escaped);
        if bold {
            out.push_str(if kind == "telegram" { "</b>" } else { "**" });
        }
    }
    out
}
fn render_unbounded(kind: &str, title: &str, body: &str) -> Rendered {
    let supported = matches!(kind, "telegram" | "mattermost");
    let title = if title.trim().is_empty() { "" } else { title };
    let rich = supported;
    let title = if rich && !title.is_empty() {
        formatted(kind, title, true)
    } else {
        title.to_owned()
    };
    let body = if rich {
        formatted(kind, body, false)
    } else {
        body.to_owned()
    };
    let text = if title.is_empty() {
        body.clone()
    } else if body.is_empty() {
        title.clone()
    } else {
        format!("{title}\n\n{body}")
    };
    Rendered {
        text,
        html: rich && kind == "telegram",
        title,
        body,
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}
fn prefix(text: &str, units: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|ch| {
            used += ch.len_utf16();
            used <= units
        })
        .collect()
}

#[derive(Clone, Copy)]
enum FieldUnit {
    Characters,
    Bytes,
}
fn field_cost(text: &str, unit: FieldUnit) -> usize {
    match unit {
        FieldUnit::Characters => text.chars().count(),
        FieldUnit::Bytes => text.len(),
    }
}
fn truncate_field(text: &str, maximum: usize, unit: FieldUnit, suffix: &str) -> String {
    if field_cost(text, unit) <= maximum {
        return text.to_owned();
    }
    let budget = maximum.saturating_sub(field_cost(suffix, unit));
    let mut used = 0;
    let prefix: String = text
        .chars()
        .take_while(|ch| {
            used += match unit {
                FieldUnit::Characters => 1,
                FieldUnit::Bytes => ch.len_utf8(),
            };
            used <= budget
        })
        .collect();
    format!("{prefix}{suffix}")
}
fn provider_fields(kind: &str, title: &str, body: &str) -> (String, String) {
    let (title_limit, body_limit, unit) = match kind {
        "pushover" => (250, 1024, FieldUnit::Characters),
        "ntfy" => (1024, 4096, FieldUnit::Bytes),
        _ => return (title.to_owned(), body.to_owned()),
    };
    (
        truncate_field(title, title_limit, unit, " [truncated]"),
        truncate_field(body, body_limit, unit, "\n\n[Message truncated]"),
    )
}
fn google_json_bytes(text: &str) -> usize {
    json!({"text": text}).to_string().len()
}
fn fit_google_source(source: &str, mut render: impl FnMut(&str) -> Rendered) -> Option<Rendered> {
    let chars: Vec<char> = source.chars().collect();
    let (mut low, mut high) = (0, chars.len());
    let mut best = None;
    while low <= high {
        let mid = low + (high - low) / 2;
        let prefix: String = chars[..mid].iter().collect();
        let candidate = render(&prefix);
        if google_json_bytes(&candidate.text) <= 32000 {
            best = Some(candidate);
            low = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    best
}

/// Truncate source text, then render again: never cut HTML tags or entities.
pub fn render(kind: &str, _cfg: &Value, title: &str, body: &str) -> Rendered {
    let (mut title, mut body) = provider_fields(kind, title, body);
    let full = render_unbounded(kind, &title, &body);
    if kind == "google_chat" {
        if google_json_bytes(&full.text) <= 32000 {
            return full;
        }
        return fit_google_source(&body, |prefix| {
            render_unbounded(kind, &title, &format!("{prefix}\n\n[Message truncated]"))
        })
        .or_else(|| {
            fit_google_source(&title, |prefix| {
                render_unbounded(kind, prefix, "[Message truncated]")
            })
        })
        .unwrap_or_else(|| render_unbounded(kind, "", "[Message truncated]"));
    }
    let limit = match kind {
        "telegram" => 4096,
        "discord" => 2000,
        _ => return full,
    };
    if utf16_len(&full.text) <= limit {
        return full;
    }
    loop {
        let marked = if body.is_empty() {
            "[Message truncated]".to_owned()
        } else {
            format!("{body}\n\n[Message truncated]")
        };
        let rendered = render_unbounded(kind, &title, &marked);
        let length = utf16_len(&rendered.text);
        if length <= limit {
            return rendered;
        }
        let remove = (length - limit).max(1);
        if !body.is_empty() {
            body = prefix(&body, utf16_len(&body).saturating_sub(remove));
        } else {
            title = prefix(&title, utf16_len(&title).saturating_sub(remove));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separate_provider_fields_use_their_documented_units() {
        let pushover = render(
            "pushover",
            &json!({}),
            &"😀".repeat(251),
            &"😀".repeat(1500),
        );
        assert_eq!(pushover.title.chars().count(), 250);
        assert_eq!(pushover.body.chars().count(), 1024);
        assert!(pushover.title.ends_with(" [truncated]"));
        assert!(pushover.body.ends_with("[Message truncated]"));
        assert!(pushover.body.len() > 1024);
        let ntfy = render("ntfy", &json!({}), &"😀".repeat(300), &"😀".repeat(2000));
        assert!(ntfy.title.len() <= 1024 && ntfy.body.len() <= 4096);
        assert!(!ntfy.title.contains('\n'));
        assert!(ntfy.body.ends_with("[Message truncated]"));
        let exact = "😀".repeat(1024);
        assert_eq!(render("pushover", &json!({}), "", &exact).body, exact);
        assert_eq!(render("ntfy", &json!({}), "", &exact).body, exact);
    }
    #[test]
    fn google_chat_limits_the_serialized_json_not_4096_characters() {
        let plain = "a".repeat(10000);
        assert_eq!(render("google_chat", &json!({}), "", &plain).text, plain);
        let escaped = render("google_chat", &json!({}), "Title", &"\0".repeat(10000));
        assert!(google_json_bytes(&escaped.text) <= 32000);
        assert!(escaped.text.ends_with("[Message truncated]"));
        assert!(escaped.text.chars().filter(|c| *c == '\0').count() > 4000);
    }
    #[test]
    fn provider_limits_preserve_unicode_and_valid_markup() {
        for kind in ["telegram", "discord"] {
            let raw = format!(
                "**{}** [link](https://example.org/path)",
                "<&😀".repeat(3000)
            );
            let rendered = render(kind, &json!({}), "Long title", &raw);
            assert!(utf16_len(&rendered.text) <= if kind == "telegram" { 4096 } else { 2000 });
            assert!(rendered.text.ends_with("[Message truncated]"));
            assert_eq!(
                rendered.text.matches("<b>").count(),
                rendered.text.matches("</b>").count()
            );
            assert_eq!(
                rendered.text.matches("<a ").count(),
                rendered.text.matches("</a>").count()
            );
            assert!(raw.len() > 10000);
        }
        let exact = "a".repeat(2000);
        assert_eq!(render("discord", &json!({}), "", &exact).text, exact);
        assert!(!render("smtp", &json!({}), "", &"x".repeat(10000))
            .text
            .contains("[Message truncated]"));
        assert!(
            render("telegram", &json!({}), &"&".repeat(6000), "")
                .text
                .len()
                <= 4096
        );
    }
    #[test]
    fn absent_titles_and_legacy_modes() {
        for payload in [
            json!({"message":"Body"}),
            json!({"title":"","message":"Body"}),
            json!({"title":"  ","message":"Body"}),
        ] {
            assert_eq!(
                render("telegram", &json!({}), event_title(&payload), "Body").text,
                "Body"
            );
        }
        assert_eq!(
            render("telegram", &json!({}), "Title", "Body").text,
            "<b>Title</b>\n\nBody"
        );
        for kind in ["telegram", "mattermost"] {
            assert_eq!(
                render(
                    kind,
                    &json!({"message_mode":"message_only"}),
                    "Title",
                    "Body"
                )
                .text,
                if kind == "telegram" {
                    "<b>Title</b>\n\nBody"
                } else {
                    "**Title**\n\nBody"
                }
            );
        }
        let payload = json!({"title":"Title"});
        assert_eq!(event_message(&payload), "");
        assert_eq!(
            render(
                "mattermost",
                &json!({}),
                event_title(&payload),
                event_message(&payload)
            )
            .text,
            "**Title**"
        );
    }
    #[test]
    fn portable_links_and_unsafe_destinations() {
        let cfg = json!({});
        let text = "Tasks:\n[TASK-123](https://tickets.example.org/project/Task/TASK-123?a=1&b=2)";
        assert_eq!(render("telegram",&cfg,"",text).text,"Tasks:\n<a href=\"https://tickets.example.org/project/Task/TASK-123?a=1&amp;b=2\">TASK-123</a>");
        assert_eq!(
            render("mattermost", &cfg, "", text).text,
            "Tasks:\n[TASK\\-123](https://tickets.example.org/project/Task/TASK-123?a=1&b=2)"
        );
        for target in [
            "javascript:alert(1)",
            "data:text/html,bad",
            "https://user:secret@example.org",
        ] {
            assert!(!render("telegram", &cfg, "", &format!("[label]({target})"))
                .text
                .contains("<a "));
        }
        assert!(
            !render("telegram", &cfg, "", r"\[literal](https://example.org)")
                .text
                .contains("<a ")
        );
    }
    #[test]
    fn portable_bold_and_literal_user_text() {
        let cfg = json!({});
        let input = "✅ **Passed** <b>unsafe</b> & \"quote\"";
        let telegram = render("telegram", &cfg, "", input);
        assert!(telegram.html);
        assert_eq!(
            telegram.text,
            "✅ <b>Passed</b> &lt;b&gt;unsafe&lt;/b&gt; &amp; &quot;quote&quot;"
        );
        assert_eq!(
            render("mattermost", &cfg, "", input).text,
            "✅ **Passed** &lt;b&gt;unsafe&lt;/b&gt; &amp; \"quote\""
        );
        assert_eq!(
            render("telegram", &cfg, "", "unmatched **bold").text,
            "unmatched **bold"
        );
        assert_eq!(
            render("telegram", &cfg, "", r"\**literal**").text,
            "**literal**"
        );
        assert_eq!(
            render(
                "telegram",
                &json!({"text_format":"portable"}),
                "<Title>",
                "Body"
            )
            .text,
            "<b>&lt;Title&gt;</b>\n\nBody"
        );
    }
}
