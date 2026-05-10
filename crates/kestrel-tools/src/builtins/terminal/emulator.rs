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

/// Semantic terminal operation emitted by the ANSI/VT parser.
#[derive(Debug, Clone, PartialEq)]
pub enum TerminalOp {
    /// Printable text run.
    Print(String),
    /// Line feed (LF).
    Linefeed,
    /// Carriage return (CR).
    CarriageReturn,
    /// Backspace (BS).
    Backspace,
    /// Horizontal tab (HT).
    Tab,
    /// Bell (BEL).
    Bell,
    /// CUU — Cursor Up.
    CursorUp(u16),
    /// CUD — Cursor Down.
    CursorDown(u16),
    /// CUF — Cursor Forward.
    CursorForward(u16),
    /// CUB — Cursor Back.
    CursorBack(u16),
    /// CUP — Cursor Position (1-based; 0 means default-to-1).
    CursorPosition { row: u16, col: u16 },
    /// CHA — Cursor Horizontal Absolute.
    CursorHorizontalAbsolute(u16),
    /// VPA — Vertical Position Absolute.
    CursorVerticalAbsolute(u16),
    /// ED — Erase in Display.
    EraseInDisplay(EraseMode),
    /// EL — Erase in Line.
    EraseInLine(EraseMode),
    /// SGR — Select Graphic Rendition (raw parameter codes).
    SetGraphicRendition(Vec<u16>),
    /// DECSC — Save Cursor.
    SaveCursor,
    /// DECRC — Restore Cursor.
    RestoreCursor,
    /// SU — Scroll Up.
    ScrollUp(u16),
    /// SD — Scroll Down.
    ScrollDown(u16),
    /// DECSTBM — Set Scrolling Region (1-based; 0 = default).
    SetScrollingRegion { top: u16, bottom: u16 },
    /// DECSET — DEC Private Mode Set (e.g. 1049 = alternate screen).
    DecPrivateModeSet(u16),
    /// DECRST — DEC Private Mode Reset.
    DecPrivateModeReset(u16),
    /// OSC 0/2 — Set Window Title.
    SetWindowTitle(String),
}

/// Erase scope for ED/EL operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EraseMode {
    /// From cursor to end.
    ToEnd,
    /// From start to cursor.
    ToStart,
    /// Entire display/line.
    All,
}

fn erase_mode_from(n: u16) -> EraseMode {
    match n {
        0 => EraseMode::ToEnd,
        1 => EraseMode::ToStart,
        _ => EraseMode::All,
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

// ─── ANSI/VT Parser ────────────────────────────────────────────────

/// Internal state for the ANSI/VT parser state machine.
#[derive(Debug)]
enum ParserState {
    /// Processing normal printable text.
    Ground,
    /// Saw ESC (0x1B).
    Escape,
    /// Collecting CSI parameters.
    Csi { buf: Vec<u8>, private: bool },
    /// Collecting OSC payload.
    Osc { buf: Vec<u8> },
    /// Saw ESC O (SS3), awaiting final byte.
    Ss3,
}

/// Incremental ANSI/VT100 parser.
///
/// Consumes raw PTY bytes and emits semantic [`TerminalOp`] values.
/// Uses a state-machine approach (inspired by tmux `input.c`) so it
/// correctly handles escape sequences split across reads.
pub struct AnsiParser {
    state: ParserState,
    /// Accumulated printable bytes awaiting flush.
    print_buf: Vec<u8>,
}

impl AnsiParser {
    pub fn new() -> Self {
        Self {
            state: ParserState::Ground,
            print_buf: Vec::with_capacity(256),
        }
    }

    /// Feed a chunk of raw PTY bytes through the parser.
    ///
    /// Returns parsed terminal operations. State persists across calls so
    /// partial escape sequences spanning reads are handled correctly.
    pub fn parse(&mut self, input: &[u8]) -> Vec<TerminalOp> {
        let mut ops = Vec::new();
        for &byte in input {
            self.process_byte(byte, &mut ops);
        }
        if matches!(self.state, ParserState::Ground) {
            if let Some(op) = self.take_print() {
                ops.push(op);
            }
        }
        ops
    }

    /// Flush remaining parser state (call on EOF/session close).
    pub fn flush(&mut self) -> Vec<TerminalOp> {
        let mut ops = Vec::new();
        if let Some(op) = self.take_print() {
            ops.push(op);
        }
        // Lossy flush of any remaining incomplete bytes.
        if !self.print_buf.is_empty() {
            let text = String::from_utf8_lossy(&self.print_buf).into_owned();
            self.print_buf.clear();
            ops.push(TerminalOp::Print(text));
        }
        ops
    }

    fn process_byte(&mut self, byte: u8, ops: &mut Vec<TerminalOp>) {
        match &self.state {
            ParserState::Ground => self.on_ground(byte, ops),
            ParserState::Escape => self.on_escape(byte, ops),
            ParserState::Csi { .. } => self.on_csi(byte, ops),
            ParserState::Osc { .. } => self.on_osc(byte, ops),
            ParserState::Ss3 => self.on_ss3(byte, ops),
        }
    }

    fn on_ground(&mut self, byte: u8, ops: &mut Vec<TerminalOp>) {
        match byte {
            0x07 => {
                if let Some(op) = self.take_print() {
                    ops.push(op);
                }
                ops.push(TerminalOp::Bell);
            }
            0x08 => {
                if let Some(op) = self.take_print() {
                    ops.push(op);
                }
                ops.push(TerminalOp::Backspace);
            }
            0x09 => {
                if let Some(op) = self.take_print() {
                    ops.push(op);
                }
                ops.push(TerminalOp::Tab);
            }
            0x0A => {
                if let Some(op) = self.take_print() {
                    ops.push(op);
                }
                ops.push(TerminalOp::Linefeed);
            }
            0x0D => {
                if let Some(op) = self.take_print() {
                    ops.push(op);
                }
                ops.push(TerminalOp::CarriageReturn);
            }
            0x1B => {
                if let Some(op) = self.take_print() {
                    ops.push(op);
                }
                self.state = ParserState::Escape;
            }
            0x00..=0x1F => {}
            _ => {
                self.print_buf.push(byte);
            }
        }
    }

    fn on_escape(&mut self, byte: u8, ops: &mut Vec<TerminalOp>) {
        match byte {
            b'[' => {
                self.state = ParserState::Csi {
                    buf: Vec::with_capacity(16),
                    private: false,
                };
            }
            b']' => {
                self.state = ParserState::Osc {
                    buf: Vec::with_capacity(64),
                };
            }
            b'7' => {
                self.state = ParserState::Ground;
                ops.push(TerminalOp::SaveCursor);
            }
            b'8' => {
                self.state = ParserState::Ground;
                ops.push(TerminalOp::RestoreCursor);
            }
            b'O' => {
                self.state = ParserState::Ss3;
            }
            b'D' => {
                self.state = ParserState::Ground;
                ops.push(TerminalOp::Linefeed); // IND — Index
            }
            b'M' => {
                self.state = ParserState::Ground;
                ops.push(TerminalOp::ScrollUp(1)); // RI — Reverse Index
            }
            b'E' => {
                self.state = ParserState::Ground;
                ops.push(TerminalOp::CarriageReturn);
                ops.push(TerminalOp::Linefeed);
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
    }

    fn on_csi(&mut self, byte: u8, ops: &mut Vec<TerminalOp>) {
        match byte {
            // Parameter bytes: digits, semicolons, '?'
            0x30..=0x3F => {
                if let ParserState::Csi {
                    ref mut buf,
                    ref mut private,
                } = self.state
                {
                    if byte == b'?' && buf.is_empty() {
                        *private = true;
                    } else {
                        buf.push(byte);
                    }
                }
            }
            // Intermediate bytes: 0x20–0x2F — skip
            0x20..=0x2F => {}
            // Final byte: 0x40–0x7E — dispatch
            0x40..=0x7E => {
                let op = match &self.state {
                    ParserState::Csi { buf, private } => {
                        if *private {
                            Self::dispatch_csi_private(buf, byte)
                        } else {
                            Self::dispatch_csi(buf, byte)
                        }
                    }
                    _ => None,
                };
                self.state = ParserState::Ground;
                if let Some(op) = op {
                    ops.push(op);
                }
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
    }

    fn on_osc(&mut self, byte: u8, ops: &mut Vec<TerminalOp>) {
        match byte {
            // BEL terminates OSC
            0x07 => {
                let op = match &self.state {
                    ParserState::Osc { buf } => Self::dispatch_osc(buf),
                    _ => None,
                };
                self.state = ParserState::Ground;
                if let Some(op) = op {
                    ops.push(op);
                }
            }
            // ESC may start ST (ESC \); terminate OSC and enter escape
            0x1B => {
                let op = match &self.state {
                    ParserState::Osc { buf } => Self::dispatch_osc(buf),
                    _ => None,
                };
                self.state = ParserState::Escape;
                if let Some(op) = op {
                    ops.push(op);
                }
            }
            _ => {
                if let ParserState::Osc { ref mut buf } = self.state {
                    buf.push(byte);
                }
            }
        }
    }

    fn on_ss3(&mut self, _byte: u8, _ops: &mut Vec<TerminalOp>) {
        self.state = ParserState::Ground;
    }

    /// Flush accumulated printable bytes as a `Print` op, keeping any
    /// trailing incomplete UTF-8 sequence in the buffer.
    fn take_print(&mut self) -> Option<TerminalOp> {
        if self.print_buf.is_empty() {
            return None;
        }
        let boundary = find_utf8_boundary(&self.print_buf);
        if boundary == 0 {
            return None;
        }
        let text_bytes = self.print_buf[..boundary].to_vec();
        self.print_buf.drain(..boundary);
        // Safety: find_utf8_boundary guarantees a valid UTF-8 prefix.
        let text = unsafe { String::from_utf8_unchecked(text_bytes) };
        Some(TerminalOp::Print(text))
    }

    fn parse_csi_params(buf: &[u8]) -> Vec<u16> {
        std::str::from_utf8(buf)
            .unwrap_or("")
            .split(';')
            .map(|s| s.parse().unwrap_or(0))
            .collect()
    }

    fn dispatch_csi(buf: &[u8], final_byte: u8) -> Option<TerminalOp> {
        let nums = Self::parse_csi_params(buf);
        match final_byte {
            b'A' => Some(TerminalOp::CursorUp(
                nums.first().copied().unwrap_or(1).max(1),
            )),
            b'B' => Some(TerminalOp::CursorDown(
                nums.first().copied().unwrap_or(1).max(1),
            )),
            b'C' => Some(TerminalOp::CursorForward(
                nums.first().copied().unwrap_or(1).max(1),
            )),
            b'D' => Some(TerminalOp::CursorBack(
                nums.first().copied().unwrap_or(1).max(1),
            )),
            b'H' | b'f' => Some(TerminalOp::CursorPosition {
                row: nums.first().copied().unwrap_or(1),
                col: nums.get(1).copied().unwrap_or(1),
            }),
            b'G' => Some(TerminalOp::CursorHorizontalAbsolute(
                nums.first().copied().unwrap_or(1),
            )),
            b'd' => Some(TerminalOp::CursorVerticalAbsolute(
                nums.first().copied().unwrap_or(1),
            )),
            b'J' => Some(TerminalOp::EraseInDisplay(erase_mode_from(
                nums.first().copied().unwrap_or(0),
            ))),
            b'K' => Some(TerminalOp::EraseInLine(erase_mode_from(
                nums.first().copied().unwrap_or(0),
            ))),
            b'm' => Some(TerminalOp::SetGraphicRendition(if nums.is_empty() {
                vec![0]
            } else {
                nums
            })),
            b's' => Some(TerminalOp::SaveCursor),
            b'u' => Some(TerminalOp::RestoreCursor),
            b'S' => Some(TerminalOp::ScrollUp(nums.first().copied().unwrap_or(1))),
            b'T' => Some(TerminalOp::ScrollDown(nums.first().copied().unwrap_or(1))),
            b'r' => Some(TerminalOp::SetScrollingRegion {
                top: nums.first().copied().unwrap_or(0),
                bottom: nums.get(1).copied().unwrap_or(0),
            }),
            _ => None,
        }
    }

    fn dispatch_csi_private(buf: &[u8], final_byte: u8) -> Option<TerminalOp> {
        let nums = Self::parse_csi_params(buf);
        match final_byte {
            b'h' => Some(TerminalOp::DecPrivateModeSet(
                nums.first().copied().unwrap_or(0),
            )),
            b'l' => Some(TerminalOp::DecPrivateModeReset(
                nums.first().copied().unwrap_or(0),
            )),
            _ => None,
        }
    }

    fn dispatch_osc(buf: &[u8]) -> Option<TerminalOp> {
        let s = std::str::from_utf8(buf).unwrap_or("");
        if let Some(pos) = s.find(';') {
            if let Ok(code) = s[..pos].parse::<u16>() {
                let content = &s[pos + 1..];
                match code {
                    0 | 2 => return Some(TerminalOp::SetWindowTitle(content.to_string())),
                    _ => {}
                }
            }
        }
        None
    }
}

impl Default for AnsiParser {
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

/// Terminal emulator handle holding the ANSI parser, screen model, and parsed operations.
///
/// The parser (#330) consumes raw PTY bytes and produces semantic
/// [`TerminalOp`] values. The screen model (#331) consumes these ops
/// to maintain a grid representation of the terminal state.
pub struct TerminalEmulatorHandle {
    /// Current session dimensions, kept in sync with PTY resizes.
    cols: u16,
    rows: u16,
    /// ANSI parser state machine.
    parser: AnsiParser,
    /// Accumulated parsed operations (consumed by screen model).
    pending_ops: Vec<TerminalOp>,
    /// Terminal screen model (grid with primary/alternate buffers).
    screen: super::screen::TerminalScreen,
}

impl TerminalEmulatorHandle {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            parser: AnsiParser::new(),
            pending_ops: Vec::new(),
            screen: super::screen::TerminalScreen::new(cols as usize, rows as usize),
        }
    }

    /// Update dimensions after a PTY resize.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.screen.resize(cols as usize, rows as usize);
    }

    #[allow(dead_code)]
    pub fn cols(&self) -> u16 {
        self.cols
    }

    #[allow(dead_code)]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Feed raw PTY bytes through the ANSI parser and update the screen model.
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        let ops = self.parser.parse(bytes);
        for op in &ops {
            self.screen.process_op(op);
        }
        self.pending_ops.extend(ops);
    }

    /// Take all pending terminal operations (consumed by screen model).
    #[allow(dead_code)]
    pub fn take_ops(&mut self) -> Vec<TerminalOp> {
        std::mem::take(&mut self.pending_ops)
    }

    /// Flush parser state (call on EOF/session close).
    pub fn flush_parser(&mut self) {
        let ops = self.parser.flush();
        for op in &ops {
            self.screen.process_op(op);
        }
        self.pending_ops.extend(ops);
    }

    /// Access the terminal screen model.
    #[allow(dead_code)]
    pub fn screen(&self) -> &super::screen::TerminalScreen {
        &self.screen
    }

    /// Access the terminal screen model mutably.
    #[allow(dead_code)]
    pub fn screen_mut(&mut self) -> &mut super::screen::TerminalScreen {
        &mut self.screen
    }

    /// Compute a fast hash of the current screen state for change detection.
    pub fn state_hash(&self) -> u64 {
        self.screen.state_hash()
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

    // ─── ANSI Parser tests ─────────────────────────────────────────

    #[test]
    fn test_parser_plain_text() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"hello world");
        assert_eq!(ops, vec![TerminalOp::Print("hello world".to_string())]);
    }

    #[test]
    fn test_parser_c0_controls() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"line1\nline2\r\t");
        assert_eq!(
            ops,
            vec![
                TerminalOp::Print("line1".to_string()),
                TerminalOp::Linefeed,
                TerminalOp::Print("line2".to_string()),
                TerminalOp::CarriageReturn,
                TerminalOp::Tab,
            ]
        );
    }

    #[test]
    fn test_parser_cursor_movement() {
        let mut p = AnsiParser::new();
        // Up 5, Down 2, Right 10, Left 3
        let ops = p.parse(b"\x1b[5A\x1b[2B\x1b[10C\x1b[3D");
        assert_eq!(
            ops,
            vec![
                TerminalOp::CursorUp(5),
                TerminalOp::CursorDown(2),
                TerminalOp::CursorForward(10),
                TerminalOp::CursorBack(3),
            ]
        );
    }

    #[test]
    fn test_parser_cursor_position() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1b[5;10H\x1b[H");
        assert_eq!(
            ops,
            vec![
                TerminalOp::CursorPosition { row: 5, col: 10 },
                TerminalOp::CursorPosition { row: 1, col: 1 },
            ]
        );
    }

    #[test]
    fn test_parser_erase_display() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1b[J\x1b[1J\x1b[2J");
        assert_eq!(
            ops,
            vec![
                TerminalOp::EraseInDisplay(EraseMode::ToEnd),
                TerminalOp::EraseInDisplay(EraseMode::ToStart),
                TerminalOp::EraseInDisplay(EraseMode::All),
            ]
        );
    }

    #[test]
    fn test_parser_erase_line() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1b[K\x1b[1K\x1b[2K");
        assert_eq!(
            ops,
            vec![
                TerminalOp::EraseInLine(EraseMode::ToEnd),
                TerminalOp::EraseInLine(EraseMode::ToStart),
                TerminalOp::EraseInLine(EraseMode::All),
            ]
        );
    }

    #[test]
    fn test_parser_sgr() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1b[31m\x1b[1;32m\x1b[0m");
        assert_eq!(
            ops,
            vec![
                TerminalOp::SetGraphicRendition(vec![31]),
                TerminalOp::SetGraphicRendition(vec![1, 32]),
                TerminalOp::SetGraphicRendition(vec![0]),
            ]
        );
    }

    #[test]
    fn test_parser_save_restore_cursor() {
        let mut p = AnsiParser::new();
        // CSI s/u and ESC 7/8
        let ops = p.parse(b"\x1b[s\x1b[u\x1b7\x1b8");
        assert_eq!(
            ops,
            vec![
                TerminalOp::SaveCursor,
                TerminalOp::RestoreCursor,
                TerminalOp::SaveCursor,
                TerminalOp::RestoreCursor,
            ]
        );
    }

    #[test]
    fn test_parser_alternate_screen() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1b[?1049h\x1b[?1049l");
        assert_eq!(
            ops,
            vec![
                TerminalOp::DecPrivateModeSet(1049),
                TerminalOp::DecPrivateModeReset(1049),
            ]
        );
    }

    #[test]
    fn test_parser_osc_title() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1b]0;My Title\x07data");
        assert_eq!(
            ops,
            vec![
                TerminalOp::SetWindowTitle("My Title".to_string()),
                TerminalOp::Print("data".to_string()),
            ]
        );
    }

    #[test]
    fn test_parser_osc_title_st() {
        let mut p = AnsiParser::new();
        // OSC terminated by ST (ESC \)
        let ops = p.parse(b"\x1b]2;title\x1b\\data");
        assert_eq!(
            ops,
            vec![
                TerminalOp::SetWindowTitle("title".to_string()),
                TerminalOp::Print("data".to_string()),
            ]
        );
    }

    #[test]
    fn test_parser_scroll() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1b[3S\x1b[2T");
        assert_eq!(
            ops,
            vec![TerminalOp::ScrollUp(3), TerminalOp::ScrollDown(2)]
        );
    }

    #[test]
    fn test_parser_scrolling_region() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1b[5;20r");
        assert_eq!(
            ops,
            vec![TerminalOp::SetScrollingRegion { top: 5, bottom: 20 }]
        );
    }

    #[test]
    fn test_parser_mixed_sequence() {
        let mut p = AnsiParser::new();
        // Typical TUI init: clear screen, set cursor, print
        let input = b"\x1b[2J\x1b[H\x1b[?1049hHello World\x1b[31mRed\x1b[0m";
        let ops = p.parse(input);
        assert_eq!(
            ops,
            vec![
                TerminalOp::EraseInDisplay(EraseMode::All),
                TerminalOp::CursorPosition { row: 1, col: 1 },
                TerminalOp::DecPrivateModeSet(1049),
                TerminalOp::Print("Hello World".to_string()),
                TerminalOp::SetGraphicRendition(vec![31]),
                TerminalOp::Print("Red".to_string()),
                TerminalOp::SetGraphicRendition(vec![0]),
            ]
        );
    }

    #[test]
    fn test_parser_split_sequence() {
        let mut p = AnsiParser::new();
        // CSI sequence split across two reads
        let ops1 = p.parse(b"\x1b[3");
        assert!(ops1.is_empty());
        let ops2 = p.parse(b"1m");
        assert_eq!(ops2, vec![TerminalOp::SetGraphicRendition(vec![31])]);
    }

    #[test]
    fn test_parser_split_text() {
        let mut p = AnsiParser::new();
        let ops1 = p.parse(b"hel");
        assert_eq!(ops1, vec![TerminalOp::Print("hel".to_string())]);
        let ops2 = p.parse(b"lo");
        assert_eq!(ops2, vec![TerminalOp::Print("lo".to_string())]);
        // Flush should emit the accumulated text
        let flush_ops = p.flush();
        assert!(flush_ops.is_empty());
    }

    #[test]
    fn test_parser_default_params() {
        let mut p = AnsiParser::new();
        // Cursor Up without param defaults to 1
        let ops = p.parse(b"\x1b[A\x1b[H");
        assert_eq!(
            ops,
            vec![
                TerminalOp::CursorUp(1),
                TerminalOp::CursorPosition { row: 1, col: 1 },
            ]
        );
    }

    #[test]
    fn test_parser_cha_vpa() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1b[20G\x1b[5d");
        assert_eq!(
            ops,
            vec![
                TerminalOp::CursorHorizontalAbsolute(20),
                TerminalOp::CursorVerticalAbsolute(5),
            ]
        );
    }

    #[test]
    fn test_parser_esc_d_m_e() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x1bD\x1bM\x1bE");
        assert_eq!(
            ops,
            vec![
                TerminalOp::Linefeed,
                TerminalOp::ScrollUp(1),
                TerminalOp::CarriageReturn,
                TerminalOp::Linefeed,
            ]
        );
    }

    #[test]
    fn test_parser_bell_backspace() {
        let mut p = AnsiParser::new();
        let ops = p.parse(b"\x07\x08");
        assert_eq!(ops, vec![TerminalOp::Bell, TerminalOp::Backspace]);
    }

    #[test]
    fn test_parser_emulator_feed() {
        let mut emu = TerminalEmulatorHandle::new(80, 24);
        emu.feed_bytes(b"\x1b[2J\x1b[1;1HHello");
        let ops = emu.take_ops();
        assert_eq!(
            ops,
            vec![
                TerminalOp::EraseInDisplay(EraseMode::All),
                TerminalOp::CursorPosition { row: 1, col: 1 },
                TerminalOp::Print("Hello".to_string()),
            ]
        );
    }

    #[test]
    fn test_parser_emulator_flush() {
        let mut emu = TerminalEmulatorHandle::new(80, 24);
        emu.feed_bytes(b"pending text");
        // Text not flushed yet (no escape sequence trigger)
        assert!(emu.take_ops().is_empty());
        // Flush forces it out
        emu.flush_parser();
        let ops = emu.take_ops();
        assert_eq!(ops, vec![TerminalOp::Print("pending text".to_string())]);
    }
}
