//! Telegram MarkdownV2 formatting helpers.

const SPECIAL_CHARS: [char; 18] = [
    '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
];

/// Convert standard Markdown to Telegram MarkdownV2 format.
/// Falls back to plain text (`None`) if conversion fails.
pub fn markdown_to_telegram(input: &str) -> Option<String> {
    let segments = split_code_segments(input)?;
    let mut output = String::new();

    for segment in segments {
        match segment {
            Segment::Text(text) => output.push_str(&convert_text_segment(&text)?),
            Segment::CodeInline(code) => {
                output.push('`');
                output.push_str(&code);
                output.push('`');
            }
            Segment::CodeBlock(code) => {
                if code.contains("```") {
                    return None;
                }

                if code.contains('\n') {
                    let (language, body) = split_code_block_language(&code);
                    output.push_str("```");
                    if let Some(language) = language {
                        output.push_str(language);
                    }
                    output.push('\n');
                    output.push_str(body);
                    if !body.ends_with('\n') {
                        output.push('\n');
                    }
                    output.push_str("```");
                } else {
                    output.push('`');
                    output.push_str(&code);
                    output.push('`');
                }
            }
        }
    }

    Some(output)
}

fn split_code_block_language(code: &str) -> (Option<&str>, &str) {
    if let Some(first_newline) = code.find('\n') {
        let prefix = &code[..first_newline];
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_alphanumeric()) {
            return (Some(prefix), &code[first_newline + 1..]);
        }
    }

    (None, code)
}

#[derive(Debug)]
enum Segment {
    Text(String),
    CodeInline(String),
    CodeBlock(String),
}

fn split_code_segments(input: &str) -> Option<Vec<Segment>> {
    let mut chars = input.char_indices().peekable();
    let mut text_start = 0usize;
    let mut segments = Vec::new();

    while let Some((idx, ch)) = chars.next() {
        if ch != '`' {
            continue;
        }

        let mut tick_count = 1usize;
        while let Some((_, '`')) = chars.peek() {
            chars.next();
            tick_count += 1;
        }

        match tick_count {
            1 => {
                if text_start < idx {
                    segments.push(Segment::Text(input[text_start..idx].to_string()));
                }
                let code_start = idx + 1;
                let rest = &input[code_start..];
                let Some(end) = rest.find('`') else {
                    continue;
                };
                let code_end = code_start + end;
                let code = &input[code_start..code_end];
                if code.contains('\n') || code.contains('`') {
                    return None;
                }
                segments.push(Segment::CodeInline(code.to_string()));
                let next_index = code_end + 1;
                while let Some((next_idx, _)) = chars.peek() {
                    if *next_idx < next_index {
                        chars.next();
                    } else {
                        break;
                    }
                }
                text_start = next_index;
            }
            3 => {
                if text_start < idx {
                    segments.push(Segment::Text(input[text_start..idx].to_string()));
                }
                let code_start = idx + 3;
                let rest = &input[code_start..];
                let end = rest.find("```")?;
                let code_end = code_start + end;
                let code = &input[code_start..code_end];
                if let Some(first_newline) = code.find('\n') {
                    let prefix = &code[..first_newline];
                    if !prefix.is_empty() && !prefix.chars().all(|c| c.is_ascii_alphanumeric()) {
                        return None;
                    }
                }
                segments.push(Segment::CodeBlock(code.to_string()));
                let next_index = code_end + 3;
                while let Some((next_idx, _)) = chars.peek() {
                    if *next_idx < next_index {
                        chars.next();
                    } else {
                        break;
                    }
                }
                text_start = next_index;
            }
            _ => continue,
        }
    }

    if text_start < input.len() {
        segments.push(Segment::Text(input[text_start..].to_string()));
    }

    Some(segments)
}

fn convert_text_segment(input: &str) -> Option<String> {
    let mut lines = Vec::new();

    for line in input.split('\n') {
        lines.push(convert_line(line)?);
    }

    Some(lines.join("\n"))
}

fn convert_line(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("## ") {
        return Some(format!("*{}*", convert_inline(rest)?));
    }

    if let Some(rest) = line.strip_prefix("- ") {
        return Some(format!("• {}", convert_inline(rest)?));
    }

    convert_inline(line)
}

fn convert_inline(input: &str) -> Option<String> {
    let mut output = String::new();
    let mut idx = 0usize;

    while idx < input.len() {
        let rest = &input[idx..];

        if let Some(after) = rest.strip_prefix("**") {
            if let Some(close) = after.find("**") {
                let inner = &after[..close];
                output.push('*');
                output.push_str(&convert_inline(inner)?);
                output.push('*');
                idx += 2 + close + 2;
                continue;
            }
        }

        if let Some(after) = rest.strip_prefix('*') {
            if let Some(close) = find_single_italic_close(after) {
                let inner = &after[..close];
                output.push('_');
                output.push_str(&convert_inline(inner)?);
                output.push('_');
                idx += 1 + close + 1;
                continue;
            }
        }

        if rest.starts_with('[') && rest.find("](").is_some_and(|close_text| close_text > 1) {
            let (rendered, consumed) = convert_link(rest)?;
            output.push_str(&rendered);
            idx += consumed;
            continue;
        }

        let ch = rest.chars().next()?;
        push_escaped(&mut output, ch);
        idx += ch.len_utf8();
    }

    Some(output)
}

fn find_single_italic_close(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut idx = 0usize;

    while idx < bytes.len() {
        if bytes[idx] == b'*' {
            let prev_is_star = idx > 0 && bytes[idx - 1] == b'*';
            let next_is_star = idx + 1 < bytes.len() && bytes[idx + 1] == b'*';
            if !prev_is_star && !next_is_star {
                return Some(idx);
            }
        }
        idx += 1;
    }

    None
}

fn convert_link(input: &str) -> Option<(String, usize)> {
    let close_text = input.find("](")?;
    let link_text = &input[1..close_text];
    let url_start = close_text + 2;
    let url_end = find_link_url_end(&input[url_start..])?;
    let url = &input[url_start..url_start + url_end];
    if link_text.is_empty() || url.is_empty() {
        return None;
    }
    let consumed = url_start + url_end + 1;

    let mut rendered = String::new();
    rendered.push('[');
    rendered.push_str(&escape_non_code(link_text));
    rendered.push_str("](");
    rendered.push_str(&escape_link_url(url));
    rendered.push(')');

    Some((rendered, consumed))
}

fn find_link_url_end(input: &str) -> Option<usize> {
    let mut depth = 0usize;

    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(idx),
            ')' => depth -= 1,
            _ => {}
        }
    }

    None
}

fn escape_non_code(input: &str) -> String {
    let mut output = String::new();
    for ch in input.chars() {
        push_escaped(&mut output, ch);
    }
    output
}

fn escape_link_url(input: &str) -> String {
    let mut output = String::new();
    for ch in input.chars() {
        if matches!(ch, ')' | '\\') {
            output.push('\\');
        }
        output.push(ch);
    }
    output
}

fn push_escaped(output: &mut String, ch: char) {
    if SPECIAL_CHARS.contains(&ch) {
        output.push('\\');
    }
    output.push(ch);
}

/// Escape HTML special characters for Telegram's HTML parse mode.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Convert Markdown to Telegram HTML format.
///
/// This is the fallback when MarkdownV2 conversion fails or Telegram rejects
/// the MarkdownV2 payload. HTML is more forgiving than MarkdownV2 and supports
/// `<b>`, `<i>`, `<code>`, `<pre>`, `<a>`, `<blockquote>` tags.
///
/// Conversion rules (common subset):
/// - `**text**` → `<b>text</b>`
/// - `*text*` → `<i>text</i>`
/// - `` `code` `` → `<code>code</code>`
/// - ` ```lang\ncode``` ` → `<pre><code class="language-lang">code</code></pre>`
/// - `## Header` → `<b>Header</b>`
/// - `- item` → `• item`
/// - `[text](url)` → `<a href="url">text</a>`
/// - All other `<`, `>`, `&` are HTML-escaped.
pub fn markdown_to_html(input: &str) -> String {
    let mut output = String::with_capacity(input.len() + 256);
    let mut remaining = input;

    while !remaining.is_empty() {
        // Fenced code block ```...```
        if let Some(after_fence) = remaining.strip_prefix("```") {
            if let Some(end) = after_fence.find("```") {
                let inner = &after_fence[..end];
                let (lang, body) = if let Some(nl) = inner.find('\n') {
                    let prefix = &inner[..nl];
                    if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_alphanumeric()) {
                        (Some(prefix), &inner[nl + 1..])
                    } else {
                        (None, inner)
                    }
                } else {
                    (None, inner)
                };
                let escaped_body = escape_html(body.trim_end_matches('\n'));
                if let Some(l) = lang {
                    output.push_str(&format!(
                        r#"<pre><code class="language-{}">{}</code></pre>"#,
                        l, escaped_body
                    ));
                } else {
                    output.push_str(&format!("<pre><code>{}</code></pre>", escaped_body));
                }
                remaining = &after_fence[end + 3..];
                continue;
            }
        }

        // Inline code `code`
        if remaining.starts_with('`') {
            let rest = &remaining[1..];
            if let Some(end) = rest.find('`') {
                let code = &rest[..end];
                output.push_str(&format!("<code>{}</code>", escape_html(code)));
                remaining = &rest[end + 1..];
                continue;
            }
        }

        // Bold **text**
        if let Some(after) = remaining.strip_prefix("**") {
            if let Some(close) = after.find("**") {
                let inner = &after[..close];
                output.push_str(&format!("<b>{}</b>", markdown_to_html(inner)));
                remaining = &after[close + 2..];
                continue;
            }
        }

        // Italic *text* (avoid matching ** which is bold)
        if remaining.starts_with('*') && !remaining.starts_with("**") {
            let rest = &remaining[1..];
            if let Some(close) = find_italic_close(rest) {
                let inner = &rest[..close];
                output.push_str(&format!("<i>{}</i>", markdown_to_html(inner)));
                remaining = &rest[close + 1..];
                continue;
            }
        }

        // Link [text](url)
        if remaining.starts_with('[') {
            if let Some(close_text) = remaining.find("](") {
                if close_text > 1 {
                    let link_text = &remaining[1..close_text];
                    let url_start = close_text + 2;
                    if let Some(url_end) = find_url_end(&remaining[url_start..]) {
                        let url = &remaining[url_start..url_start + url_end];
                        output.push_str(&format!(
                            r#"<a href="{}">{}</a>"#,
                            escape_html(url),
                            escape_html(link_text)
                        ));
                        remaining = &remaining[url_start + url_end + 1..];
                        continue;
                    }
                }
            }
        }

        // Header ## text (line-level)
        if let Some(rest) = remaining.strip_prefix("## ") {
            if let Some(nl) = rest.find('\n') {
                output.push_str(&format!("<b>{}</b>\n", escape_html(&rest[..nl])));
                remaining = &rest[nl..];
                continue;
            } else {
                output.push_str(&format!("<b>{}</b>", escape_html(rest)));
                break;
            }
        }

        // Single-char: escape if needed, advance one char
        let ch = remaining.chars().next().unwrap();
        match ch {
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '&' => output.push_str("&amp;"),
            _ => output.push(ch),
        }
        remaining = &remaining[ch.len_utf8()..];
    }

    output
}

/// Find the closing `*` for italic, skipping `**` (bold markers).
fn find_italic_close(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'*' {
            let prev_is_star = idx > 0 && bytes[idx - 1] == b'*';
            let next_is_star = idx + 1 < bytes.len() && bytes[idx + 1] == b'*';
            if !prev_is_star && !next_is_star {
                return Some(idx);
            }
        }
        idx += 1;
    }
    None
}

/// Find the closing `)` for a URL in `[text](url)`.
fn find_url_end(input: &str) -> Option<usize> {
    for (idx, ch) in input.char_indices() {
        if ch == ')' {
            return Some(idx);
        }
    }
    None
}

/// Strip all Markdown formatting to produce clean plain text.
///
/// This is the last-resort fallback when both MarkdownV2 and HTML fail.
/// It removes formatting markers while preserving readable text.
pub fn strip_markdown(input: &str) -> String {
    let mut result = input.to_string();

    // Remove code fences but keep content
    result = result.replace("```", "");

    // Remove bold/italic markers
    result = result.replace("**", "");
    // Single * that aren't list markers — just remove the asterisk
    // Be conservative: only remove * that are clearly emphasis (preceded/followed by word char)
    result = result.replace('*', "");

    // Remove header markers
    result = result.replace("## ", "");
    result = result.replace("# ", "");

    // Remove link syntax, keep text: [text](url) → text
    while let Some(start) = result.find('[') {
        if let Some(close_text) = result[start..].find("](") {
            if let Some(url_end) = result[start + close_text + 2..].find(')') {
                let text = &result[start + 1..start + close_text];
                let after = &result[start + close_text + 2 + url_end + 1..];
                result = format!("{}{}{}", &result[..start], text, after);
                continue;
            }
        }
        break; // Malformed link, stop
    }

    // Remove strikethrough markers
    result = result.replace("~~", "");

    // Remove inline code backticks
    result = result.replace('`', "");

    result
}

#[cfg(test)]
mod tests {
    use super::{markdown_to_html, markdown_to_telegram, strip_markdown};

    #[test]
    fn test_bold_and_italic_conversion() {
        let formatted = markdown_to_telegram("**bold** and *italic*").unwrap();
        assert_eq!(formatted, "*bold* and _italic_");
    }

    #[test]
    fn test_code_block_preserved_without_escaping() {
        let formatted = markdown_to_telegram("Before\n```let x = a_b;```\nAfter").unwrap();
        assert_eq!(formatted, "Before\n`let x = a_b;`\nAfter");
    }

    #[test]
    fn test_special_character_escaping() {
        let formatted = markdown_to_telegram("_*[]()~>#+-=|{}.!").unwrap();
        assert_eq!(
            formatted,
            "\\_\\*\\[\\]\\(\\)\\~\\>\\#\\+\\-\\=\\|\\{\\}\\.\\!"
        );
    }

    #[test]
    fn test_invalid_input_falls_back() {
        assert!(markdown_to_telegram("[broken](https://example.com").is_none());
        assert!(markdown_to_telegram("```unterminated").is_none());
    }

    #[test]
    fn test_header_conversion() {
        let formatted = markdown_to_telegram("## Heading").unwrap();
        assert_eq!(formatted, "*Heading*");
    }

    #[test]
    fn test_list_marker_conversion() {
        let formatted = markdown_to_telegram("- item").unwrap();
        assert_eq!(formatted, "• item");
    }

    #[test]
    fn test_mixed_content_conversion() {
        let formatted =
            markdown_to_telegram("**Title** uses `code` and [link](https://example.com/a)")
                .unwrap();
        assert_eq!(
            formatted,
            "*Title* uses `code` and [link](https://example.com/a)"
        );
    }

    #[test]
    fn test_multiline_fenced_code_block_stays_fenced() {
        let formatted = markdown_to_telegram("```rust\nlet x = 1;\nlet y = x + 1;\n```").unwrap();
        assert_eq!(formatted, "```rust\nlet x = 1;\nlet y = x + 1;\n```");
    }

    // ── markdown_to_html tests ──────────────────────────────

    #[test]
    fn test_html_bold_and_italic() {
        let html = markdown_to_html("**bold** and *italic*");
        assert_eq!(html, "<b>bold</b> and <i>italic</i>");
    }

    #[test]
    fn test_html_code_block() {
        let html = markdown_to_html("```rust\nlet x = 1;\n```");
        assert!(html.contains("<pre><code"));
        assert!(html.contains("let x = 1;"));
    }

    #[test]
    fn test_html_inline_code() {
        let html = markdown_to_html("Use `git status` to check");
        assert_eq!(html, "Use <code>git status</code> to check");
    }

    #[test]
    fn test_html_header() {
        let html = markdown_to_html("## My Heading\nNext line");
        assert!(html.starts_with("<b>My Heading</b>"));
    }

    #[test]
    fn test_html_link() {
        let html = markdown_to_html("[click here](https://example.com)");
        assert_eq!(html, r#"<a href="https://example.com">click here</a>"#);
    }

    #[test]
    fn test_html_escapes_special() {
        let html = markdown_to_html("a < b > c & d");
        assert_eq!(html, "a &lt; b &gt; c &amp; d");
    }

    #[test]
    fn test_html_table_content_preserved() {
        // Tables don't have special HTML handling — pipe chars pass through
        let html = markdown_to_html("| col1 | col2 |");
        assert!(html.contains("col1"));
        assert!(html.contains("col2"));
    }

    // ── strip_markdown tests ────────────────────────────────

    #[test]
    fn test_strip_bold_italic() {
        assert_eq!(strip_markdown("**bold** and *italic*"), "bold and italic");
    }

    #[test]
    fn test_strip_code() {
        assert_eq!(strip_markdown("Use `code` here"), "Use code here");
    }

    #[test]
    fn test_strip_header() {
        assert_eq!(strip_markdown("## Heading"), "Heading");
    }

    #[test]
    fn test_strip_link() {
        assert_eq!(strip_markdown("[text](https://example.com)"), "text");
    }

    #[test]
    fn test_strip_code_block() {
        assert_eq!(
            strip_markdown("```rust\nlet x = 1;\n```"),
            "rust\nlet x = 1;\n"
        );
    }
}
