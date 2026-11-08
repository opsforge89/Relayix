//! Delivery presentation is independent from the event title used in history.
use crate::*;

pub const TITLE_SQL:&str="COALESCE(CASE WHEN jsonb_typeof(COALESCE(e.rendered_payload,e.payload)->'title')='string' AND COALESCE(e.rendered_payload,e.payload)->>'title' ~ '[^[:space:]]' THEN COALESCE(e.rendered_payload,e.payload)->>'title' END,CASE WHEN jsonb_typeof(COALESCE(e.rendered_payload,e.payload)->'alertname')='string' AND COALESCE(e.rendered_payload,e.payload)->>'alertname' ~ '[^[:space:]]' THEN COALESCE(e.rendered_payload,e.payload)->>'alertname' END,'')";

pub struct Rendered {
    pub text: String,
    pub html: bool,
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
pub fn render(kind: &str, _cfg: &Value, title: &str, body: &str) -> Rendered {
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
        body
    } else if body.is_empty() {
        title
    } else {
        format!("{title}\n\n{body}")
    };
    Rendered {
        text,
        html: rich && kind == "telegram",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
