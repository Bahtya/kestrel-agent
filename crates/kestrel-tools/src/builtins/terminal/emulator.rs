//! Internal types for the terminal emulator layer.
//!
//! This module provides the scaffolding for full terminal emulation. Currently
//! it defines the output layer types and a placeholder emulator handle that
//! future issues (#330, #331) will flesh out with an ANSI/VT parser and
//! screen/grid model.

/// Read mode for `terminal_read_output`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadMode {
    /// Raw bytes converted to lossy UTF-8 (preserves ANSI sequences).
    Raw,
    /// Control characters are escaped for visibility (e.g. `\x1b` shown as `<ESC>`).
    Escaped,
    /// Strip non-printable control sequences, returning only visible text.
    Text,
}

impl ReadMode {
    pub fn parse_mode(s: &str) -> Option<Self> {
        match s {
            "raw" => Some(Self::Raw),
            "escaped" => Some(Self::Escaped),
            "text" => Some(Self::Text),
            _ => None,
        }
    }
}

/// Incremental UTF-8 decoder that preserves incomplete multibyte tails
/// across PTY reads.
///
/// PTY `read()` calls can split a multi-byte UTF-8 sequence at arbitrary
/// boundaries. If we convert each chunk independently with
/// `String::from_utf8_lossy()`, the leading/trailing fragments become `�`.
/// This decoder keeps the incomplete tail bytes and prepends them to the next
/// chunk, ensuring correct decoding.
pub struct IncrementalUtf8Decoder {
    /// Incomplete UTF-8 tail from the previous PTY read (1–3 bytes).
    pending: Vec<u8>,
}

impl IncrementalUtf8Decoder {
    pub fn new() -> Self {
        Self {
            pending: Vec::with_capacity(3),
        }
    }

    /// Decode a new chunk of PTY bytes.
    ///
    /// Returns a `String` of the fully decoded content. Any trailing bytes
    /// that form an incomplete UTF-8 sequence are held internally for the
    /// next call.
    pub fn decode(&mut self, chunk: &[u8]) -> String {
        if chunk.is_empty() && self.pending.is_empty() {
            return String::new();
        }

        // Prepend any leftover bytes from the previous read.
        let mut combined: Vec<u8>;
        let input: &[u8] = if self.pending.is_empty() {
            chunk
        } else {
            combined = Vec::with_capacity(self.pending.len() + chunk.len());
            combined.extend_from_slice(&self.pending);
            combined.extend_from_slice(chunk);
            self.pending.clear();
            &combined
        };

        if input.is_empty() {
            return String::new();
        }

        // Find the longest valid UTF-8 prefix.
        let split_point = find_utf8_boundary(input);
        if split_point == input.len() {
            // Entire input is valid UTF-8.
            // Safety: we verified the entire slice is valid UTF-8.
            unsafe { String::from_utf8_unchecked(input.to_vec()) }
        } else {
            // Save the trailing incomplete bytes.
            self.pending.extend_from_slice(&input[split_point..]);
            // Safety: we found the boundary where valid UTF-8 ends.
            unsafe { String::from_utf8_unchecked(input[..split_point].to_vec()) }
        }
    }

    /// Flush any remaining pending bytes as lossy UTF-8.
    ///
    /// Should be called when the PTY stream ends (EOF) to avoid silently
    /// discarding leftover bytes that never completed a character.
    pub fn flush_lossy(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let s = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        s
    }

    /// Whether there are pending bytes waiting for completion.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

impl Default for IncrementalUtf8Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Find the byte index of the longest valid UTF-8 prefix in `input`.
///
/// Returns `input.len()` if the entire slice is valid UTF-8.
/// Otherwise returns the index of the last byte before an incomplete sequence.
fn find_utf8_boundary(input: &[u8]) -> usize {
    match std::str::from_utf8(input) {
        Ok(_) => input.len(),
        Err(e) => {
            let valid_up_to = e.valid_up_to();
            let error_len = e.error_len();

            match error_len {
                // Unexpected byte — everything before it is valid.
                None => valid_up_to,
                // The error starts a multi-byte sequence that is incomplete.
                // If it's at the very end, the sequence *might* complete with
                // the next chunk — so exclude it.
                Some(1..=3) => {
                    // Check if the error is at the tail of the input — meaning
                    // the sequence might be completed by the next read.
                    if valid_up_to + error_len.unwrap() >= input.len() {
                        // Trailing incomplete sequence — don't include it.
                        valid_up_to
                    } else {
                        // Unexpected byte in the middle — treat everything up
                        // to the error as valid and let lossy handle the rest.
                        valid_up_to
                    }
                }
                Some(_) => valid_up_to,
            }
        }
    }
}

/// Placeholder handle for the future terminal emulator.
///
/// Issues #330 (ANSI/VT parser) and #331 (screen/grid model) will populate
/// this with actual parsing and screen state. For now it exists so the
/// session can hold an `Option<TerminalEmulatorHandle>` without type changes
/// later.
pub struct TerminalEmulatorHandle {
    /// Current session dimensions, kept in sync with PTY resizes.
    cols: u16,
    rows: u16,
}

impl TerminalEmulatorHandle {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }

    /// Update dimensions after a PTY resize.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
    }

    #[allow(dead_code)]
    pub fn cols(&self) -> u16 {
        self.cols
    }

    #[allow(dead_code)]
    pub fn rows(&self) -> u16 {
        self.rows
    }
}

/// Strip ANSI/VT control sequences from a string, returning only visible text.
///
/// Handles:
/// - CSI sequences (`ESC [ ... <final byte>`)
/// - OSC sequences (`ESC ] ... BEL` or `ESC ] ... ST`)
/// - Simple ESC sequences (`ESC <byte>`)
/// - Other C0 controls (except common whitespace like `\n`, `\r`, `\t`)
pub fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\x1b' => {
                // Escape sequence
                if i + 1 >= bytes.len() {
                    break;
                }
                match bytes[i + 1] {
                    b'[' => {
                        // CSI: skip until final byte (0x40..=0x7E)
                        i += 2;
                        while i < bytes.len() && !(bytes[i] >= 0x40 && bytes[i] <= 0x7E) {
                            i += 1;
                        }
                        if i < bytes.len() {
                            i += 1; // skip the final byte
                        }
                    }
                    b']' => {
                        // OSC: skip until BEL (0x07) or ST (ESC \)
                        i += 2;
                        while i < bytes.len() {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == b'\\' && i > 0 && bytes[i - 1] == b'\x1b' {
                                i += 1;
                                break;
                            }
                            i += 1;
                        }
                    }
                    _ => {
                        // Simple ESC sequence (2 bytes)
                        i += 2;
                    }
                }
            }
            // Keep common whitespace
            b'\n' | b'\r' | b'\t' => {
                result.push(bytes[i] as char);
                i += 1;
            }
            // Drop other C0 controls
            0x00..=0x1F => {
                i += 1;
            }
            // Printable ASCII or UTF-8 lead byte — keep as-is
            _ => {
                // Find the end of this character (for multi-byte UTF-8)
                let char_len = utf8_char_len(bytes[i]);
                let end = (i + char_len).min(bytes.len());
                if end <= bytes.len() {
                    result.push_str(&s[i..end]);
                }
                i = end;
            }
        }
    }

    result
}

/// Escape ANSI/control bytes for debug visibility.
///
/// Replaces ESC with `<ESC>`, other C0 controls with `<XX>` hex notation,
/// and keeps everything else as-is.
pub fn escape_control(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\x1b' => {
                result.push_str("<ESC>");
                i += 1;
            }
            b'\n' => {
                result.push_str("\\n");
                i += 1;
            }
            b'\r' => {
                result.push_str("\\r");
                i += 1;
            }
            b'\t' => {
                result.push_str("\\t");
                i += 1;
            }
            0x00..=0x1F => {
                result.push_str(&format!("<{:02X}>", bytes[i]));
                i += 1;
            }
            _ => {
                let char_len = utf8_char_len(bytes[i]);
                let end = (i + char_len).min(bytes.len());
                if end <= bytes.len() {
                    result.push_str(&s[i..end]);
                }
                i = end;
            }
        }
    }

    result
}

/// Return the expected length of a UTF-8 character starting with the given lead byte.
fn utf8_char_len(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead < 0xE0 {
        2
    } else if lead < 0xF0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_mode_parse_mode() {
        assert_eq!(ReadMode::parse_mode("raw"), Some(ReadMode::Raw));
        assert_eq!(ReadMode::parse_mode("escaped"), Some(ReadMode::Escaped));
        assert_eq!(ReadMode::parse_mode("text"), Some(ReadMode::Text));
        assert_eq!(ReadMode::parse_mode("other"), None);
    }

    #[test]
    fn test_incremental_utf8_ascii() {
        let mut dec = IncrementalUtf8Decoder::new();
        assert_eq!(dec.decode(b"hello"), "hello");
        assert!(!dec.has_pending());
    }

    #[test]
    fn test_incremental_utf8_split_multibyte() {
        let mut dec = IncrementalUtf8Decoder::new();

        // '中' is E4 B8 AD in UTF-8 (3 bytes)
        // First read gets 2 bytes, second read gets the 3rd byte
        assert_eq!(dec.decode(&[0xE4, 0xB8]), "");
        assert!(dec.has_pending());
        assert_eq!(dec.decode(&[0xAD]), "中");
        assert!(!dec.has_pending());
    }

    #[test]
    fn test_incremental_utf8_split_emoji() {
        let mut dec = IncrementalUtf8Decoder::new();

        // '😀' is F0 9F 98 80 in UTF-8 (4 bytes)
        assert_eq!(dec.decode(&[0xF0, 0x9F]), "");
        assert_eq!(dec.decode(&[0x98, 0x80]), "😀");
        assert!(!dec.has_pending());
    }

    #[test]
    fn test_incremental_utf8_mixed() {
        let mut dec = IncrementalUtf8Decoder::new();

        // "hi中" = 68 69 E4 B8 AD
        assert_eq!(dec.decode(b"hi\xE4"), "hi");
        assert_eq!(dec.decode(b"\xB8\xADbye"), "中bye");
    }

    #[test]
    fn test_incremental_utf8_no_split() {
        let mut dec = IncrementalUtf8Decoder::new();
        assert_eq!(dec.decode("你好世界".as_bytes()), "你好世界");
    }

    #[test]
    fn test_incremental_utf8_flush_lossy() {
        let mut dec = IncrementalUtf8Decoder::new();
        // Incomplete 3-byte sequence, never completed
        dec.decode(&[0xE4, 0xB8]);
        let flushed = dec.flush_lossy();
        assert!(!flushed.is_empty()); // Should produce replacement char
        assert!(!dec.has_pending());
    }

    #[test]
    fn test_incremental_utf8_empty_chunks() {
        let mut dec = IncrementalUtf8Decoder::new();
        assert_eq!(dec.decode(b""), "");
        assert_eq!(dec.decode(b"hello"), "hello");
        assert_eq!(dec.decode(b""), "");
    }

    #[test]
    fn test_strip_ansi_csi() {
        let input = "\x1b[31mHello\x1b[0m World";
        assert_eq!(strip_ansi(input), "Hello World");
    }

    #[test]
    fn test_strip_ansi_cursor_move() {
        let input = "\x1b[2J\x1b[H\x1b[1;1HHello";
        assert_eq!(strip_ansi(input), "Hello");
    }

    #[test]
    fn test_strip_ansi_osc() {
        let input = "\x1b]0;title\x07Content";
        assert_eq!(strip_ansi(input), "Content");
    }

    #[test]
    fn test_strip_ansi_preserves_newlines() {
        let input = "line1\nline2\r\nline3";
        assert_eq!(strip_ansi(input), "line1\nline2\r\nline3");
    }

    #[test]
    fn test_strip_ansi_no_sequences() {
        let input = "plain text";
        assert_eq!(strip_ansi(input), "plain text");
    }

    #[test]
    fn test_strip_ansi_multibyte() {
        let input = "\x1b[32m你好\x1b[0m";
        assert_eq!(strip_ansi(input), "你好");
    }

    #[test]
    fn test_escape_control_esc() {
        assert_eq!(escape_control("\x1b["), "<ESC>[");
    }

    #[test]
    fn test_escape_control_newlines() {
        assert_eq!(escape_control("a\nb\rc"), "a\\nb\\rc");
    }

    #[test]
    fn test_escape_control_other_c0() {
        assert_eq!(escape_control("\x00\x01\x1F"), "<00><01><1F>");
    }

    #[test]
    fn test_escape_control_mixed() {
        let input = "hi\x1b[31mred\x1b[0m";
        assert_eq!(escape_control(input), "hi<ESC>[31mred<ESC>[0m");
    }

    #[test]
    fn test_emulator_handle_dims() {
        let mut handle = TerminalEmulatorHandle::new(80, 24);
        assert_eq!(handle.cols(), 80);
        assert_eq!(handle.rows(), 24);
        handle.resize(120, 40);
        assert_eq!(handle.cols(), 120);
        assert_eq!(handle.rows(), 40);
    }
}
