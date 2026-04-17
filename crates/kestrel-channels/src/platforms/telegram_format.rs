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

#[cfg(test)]
mod tests {
    use super::markdown_to_telegram;

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
}
