//! ANSI escape sequence parser for terminal input.
//!
//! Implements a state machine that processes raw bytes from the serial console
//! and converts multi-byte escape sequences (e.g., arrow keys, Home, End,
//! Delete) into discrete [`AnsiEvent`] values.
//!
//! Supports CSI sequences (`ESC [`) with numeric parameters and extended
//! function keys (`ESC [ n ~`).

/// Events produced by the ANSI parser after processing one or more input bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiEvent {
    /// A regular printable character.
    Char(u8),
    /// Arrow up (history previous).
    ArrowUp,
    /// Arrow down (history next).
    ArrowDown,
    /// Arrow left (cursor left).
    ArrowLeft,
    /// Arrow right (cursor right).
    ArrowRight,
    /// Home key (move to start of line).
    Home,
    /// End key (move to end of line).
    End,
    /// Delete key (delete char under cursor).
    Delete,
    /// Insert key (toggle insert/overwrite mode).
    Insert,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Backspace (0x7F or 0x08).
    Backspace,
    /// Enter / Return.
    Enter,
    /// Tab key.
    Tab,
    /// Ctrl-C (interrupt).
    CtrlC,
    /// Ctrl-D (EOF).
    CtrlD,
    /// Ctrl-L (clear screen).
    CtrlL,
    /// Ctrl-Z (suspend).
    CtrlZ,
    /// Ctrl-A (beginning of line).
    CtrlA,
    /// Ctrl-E (end of line).
    CtrlE,
    /// Ctrl-K (kill to end of line).
    CtrlK,
    /// Ctrl-U (kill to beginning of line).
    CtrlU,
    /// Ctrl-W (kill previous word).
    CtrlW,
    /// Unrecognized or incomplete sequence.
    Unknown,
}

/// Internal parser states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Normal character input.
    Normal,
    /// Received ESC (0x1B), waiting for `[` or other.
    Escape,
    /// Inside a CSI sequence (`ESC [`), collecting numeric parameters.
    CsiParam,
}

/// ANSI escape sequence parser.
///
/// Feed bytes one at a time via [`feed`](AnsiParser::feed). When a complete
/// event is recognized, `feed` returns `Some(AnsiEvent)`.
pub struct AnsiParser {
    state: State,
    /// Accumulated numeric parameter (e.g., the `3` in `ESC[3~`).
    param: u16,
    /// Whether we have seen any digit in the current parameter.
    has_param: bool,
}

impl AnsiParser {
    /// Create a new parser in the normal (ground) state.
    pub const fn new() -> Self {
        Self {
            state: State::Normal,
            param: 0,
            has_param: false,
        }
    }

    /// Feed one byte and optionally receive a completed event.
    ///
    /// Returns `None` when more bytes are needed to complete a sequence.
    pub fn feed(&mut self, byte: u8) -> Option<AnsiEvent> {
        match self.state {
            State::Normal => self.feed_normal(byte),
            State::Escape => self.feed_escape(byte),
            State::CsiParam => self.feed_csi(byte),
        }
    }

    /// Reset the parser to the ground state.
    fn reset(&mut self) {
        self.state = State::Normal;
        self.param = 0;
        self.has_param = false;
    }

    fn feed_normal(&mut self, byte: u8) -> Option<AnsiEvent> {
        match byte {
            0x1B => {
                // ESC — start of escape sequence
                self.state = State::Escape;
                None
            }
            b'\r' | b'\n' => Some(AnsiEvent::Enter),
            0x7F | 0x08 => Some(AnsiEvent::Backspace),
            b'\t' => Some(AnsiEvent::Tab),
            1 => Some(AnsiEvent::CtrlA),  // Ctrl-A
            3 => Some(AnsiEvent::CtrlC),  // Ctrl-C
            4 => Some(AnsiEvent::CtrlD),  // Ctrl-D
            5 => Some(AnsiEvent::CtrlE),  // Ctrl-E
            11 => Some(AnsiEvent::CtrlK), // Ctrl-K
            12 => Some(AnsiEvent::CtrlL), // Ctrl-L
            21 => Some(AnsiEvent::CtrlU), // Ctrl-U
            23 => Some(AnsiEvent::CtrlW), // Ctrl-W
            26 => Some(AnsiEvent::CtrlZ), // Ctrl-Z
            ch if (0x20..0x7F).contains(&ch) => Some(AnsiEvent::Char(ch)),
            _ => None, // ignore other control chars
        }
    }

    fn feed_escape(&mut self, byte: u8) -> Option<AnsiEvent> {
        match byte {
            b'[' => {
                // CSI sequence start
                self.state = State::CsiParam;
                self.param = 0;
                self.has_param = false;
                None
            }
            _ => {
                // Unrecognized escape — emit ESC as unknown and reprocess byte
                self.reset();
                // In a simple parser we just drop the ESC + unrecognized byte
                Some(AnsiEvent::Unknown)
            }
        }
    }

    fn feed_csi(&mut self, byte: u8) -> Option<AnsiEvent> {
        match byte {
            b'0'..=b'9' => {
                // Accumulate numeric parameter
                self.param = self
                    .param
                    .saturating_mul(10)
                    .saturating_add((byte - b'0') as u16);
                self.has_param = true;
                None
            }
            b'A' => {
                self.reset();
                Some(AnsiEvent::ArrowUp)
            }
            b'B' => {
                self.reset();
                Some(AnsiEvent::ArrowDown)
            }
            b'C' => {
                self.reset();
                Some(AnsiEvent::ArrowRight)
            }
            b'D' => {
                self.reset();
                Some(AnsiEvent::ArrowLeft)
            }
            b'H' => {
                self.reset();
                Some(AnsiEvent::Home)
            }
            b'F' => {
                self.reset();
                Some(AnsiEvent::End)
            }
            b'~' => {
                // Extended keys: ESC [ n ~
                let event = match self.param {
                    2 => AnsiEvent::Insert,
                    3 => AnsiEvent::Delete,
                    5 => AnsiEvent::PageUp,
                    6 => AnsiEvent::PageDown,
                    1 | 7 => AnsiEvent::Home,
                    4 | 8 => AnsiEvent::End,
                    _ => AnsiEvent::Unknown,
                };
                self.reset();
                Some(event)
            }
            b';' => {
                // Modifier separator (e.g., ESC[1;5C for Ctrl-Right).
                // We ignore modifiers for now and continue parsing.
                self.param = 0;
                self.has_param = false;
                None
            }
            _ => {
                // Unrecognized CSI final byte
                self.reset();
                Some(AnsiEvent::Unknown)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_printable_char() {
        let mut parser = AnsiParser::new();
        assert_eq!(parser.feed(b'a'), Some(AnsiEvent::Char(b'a')));
        assert_eq!(parser.feed(b'Z'), Some(AnsiEvent::Char(b'Z')));
        assert_eq!(parser.feed(b' '), Some(AnsiEvent::Char(b' ')));
    }

    #[test]
    fn test_enter() {
        let mut parser = AnsiParser::new();
        assert_eq!(parser.feed(b'\r'), Some(AnsiEvent::Enter));
        assert_eq!(parser.feed(b'\n'), Some(AnsiEvent::Enter));
    }

    #[test]
    fn test_backspace() {
        let mut parser = AnsiParser::new();
        assert_eq!(parser.feed(0x7F), Some(AnsiEvent::Backspace));
        assert_eq!(parser.feed(0x08), Some(AnsiEvent::Backspace));
    }

    #[test]
    fn test_ctrl_keys() {
        let mut parser = AnsiParser::new();
        assert_eq!(parser.feed(1), Some(AnsiEvent::CtrlA));
        assert_eq!(parser.feed(3), Some(AnsiEvent::CtrlC));
        assert_eq!(parser.feed(4), Some(AnsiEvent::CtrlD));
        assert_eq!(parser.feed(5), Some(AnsiEvent::CtrlE));
        assert_eq!(parser.feed(11), Some(AnsiEvent::CtrlK));
        assert_eq!(parser.feed(12), Some(AnsiEvent::CtrlL));
        assert_eq!(parser.feed(21), Some(AnsiEvent::CtrlU));
        assert_eq!(parser.feed(23), Some(AnsiEvent::CtrlW));
        assert_eq!(parser.feed(26), Some(AnsiEvent::CtrlZ));
    }

    #[test]
    fn test_arrow_keys() {
        let mut parser = AnsiParser::new();
        // ESC [ A = Arrow Up
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'A'), Some(AnsiEvent::ArrowUp));

        // ESC [ B = Arrow Down
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'B'), Some(AnsiEvent::ArrowDown));

        // ESC [ C = Arrow Right
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'C'), Some(AnsiEvent::ArrowRight));

        // ESC [ D = Arrow Left
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'D'), Some(AnsiEvent::ArrowLeft));
    }

    #[test]
    fn test_home_end() {
        let mut parser = AnsiParser::new();
        // ESC [ H = Home
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'H'), Some(AnsiEvent::Home));

        // ESC [ F = End
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'F'), Some(AnsiEvent::End));
    }

    #[test]
    fn test_extended_keys() {
        let mut parser = AnsiParser::new();
        // ESC [ 3 ~ = Delete
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'3'), None);
        assert_eq!(parser.feed(b'~'), Some(AnsiEvent::Delete));

        // ESC [ 2 ~ = Insert
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'2'), None);
        assert_eq!(parser.feed(b'~'), Some(AnsiEvent::Insert));

        // ESC [ 5 ~ = PageUp
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'5'), None);
        assert_eq!(parser.feed(b'~'), Some(AnsiEvent::PageUp));

        // ESC [ 6 ~ = PageDown
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'6'), None);
        assert_eq!(parser.feed(b'~'), Some(AnsiEvent::PageDown));
    }

    #[test]
    fn test_tab() {
        let mut parser = AnsiParser::new();
        assert_eq!(parser.feed(b'\t'), Some(AnsiEvent::Tab));
    }

    #[test]
    fn test_interleaved_normal_and_escape() {
        let mut parser = AnsiParser::new();
        assert_eq!(parser.feed(b'h'), Some(AnsiEvent::Char(b'h')));
        assert_eq!(parser.feed(0x1B), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'A'), Some(AnsiEvent::ArrowUp));
        assert_eq!(parser.feed(b'i'), Some(AnsiEvent::Char(b'i')));
    }
}
