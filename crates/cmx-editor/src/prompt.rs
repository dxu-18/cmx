//! Single-line text input (used by the status-line prompt and the palette).

use cmx_tui::unicode::{next_boundary, prev_boundary, str_width};
use cmx_tui::{KeyCode, KeyEvent};
use std::path::PathBuf;

#[derive(Default, Clone, Debug)]
pub struct LineInput {
    pub text: String,
    pub cursor: usize,
}

#[derive(PartialEq, Eq, Debug)]
pub enum InputResult {
    Edited,
    Moved,
    Submit,
    Cancel,
    Ignored,
}

impl LineInput {
    pub fn with(text: &str) -> LineInput {
        LineInput { text: text.to_string(), cursor: text.len() }
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = text.len();
    }

    pub fn insert(&mut self, s: &str) {
        let s: String = s.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
    }

    pub fn handle(&mut self, k: &KeyEvent) -> InputResult {
        let m = k.mods;
        match k.code {
            KeyCode::Enter => InputResult::Submit,
            KeyCode::Esc => InputResult::Cancel,
            KeyCode::Char('c') if m.ctrl() => InputResult::Cancel,
            KeyCode::Char('u') if m.ctrl() => {
                self.text.drain(..self.cursor);
                self.cursor = 0;
                InputResult::Edited
            }
            KeyCode::Char('w') if m.ctrl() => {
                self.delete_word();
                InputResult::Edited
            }
            KeyCode::Backspace if m.ctrl() || m.alt() => {
                self.delete_word();
                InputResult::Edited
            }
            KeyCode::Char('a') if m.ctrl() => {
                self.cursor = 0;
                InputResult::Moved
            }
            KeyCode::Char('e') if m.ctrl() => {
                self.cursor = self.text.len();
                InputResult::Moved
            }
            KeyCode::Char(c) if !m.ctrl() && !m.alt() && !m.sup() => {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                InputResult::Edited
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let p = prev_boundary(&self.text, self.cursor);
                    self.text.drain(p..self.cursor);
                    self.cursor = p;
                    InputResult::Edited
                } else {
                    InputResult::Ignored
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.text.len() {
                    let n = next_boundary(&self.text, self.cursor);
                    self.text.drain(self.cursor..n);
                    InputResult::Edited
                } else {
                    InputResult::Ignored
                }
            }
            KeyCode::Left => {
                if m.ctrl() || m.alt() {
                    self.word_left();
                } else if self.cursor > 0 {
                    self.cursor = prev_boundary(&self.text, self.cursor);
                }
                InputResult::Moved
            }
            KeyCode::Right => {
                if m.ctrl() || m.alt() {
                    self.word_right();
                } else if self.cursor < self.text.len() {
                    self.cursor = next_boundary(&self.text, self.cursor);
                }
                InputResult::Moved
            }
            KeyCode::Home => {
                self.cursor = 0;
                InputResult::Moved
            }
            KeyCode::End => {
                self.cursor = self.text.len();
                InputResult::Moved
            }
            _ => InputResult::Ignored,
        }
    }

    fn delete_word(&mut self) {
        let end = self.cursor;
        self.word_left();
        self.text.drain(self.cursor..end);
    }

    fn word_left(&mut self) {
        let s = &self.text[..self.cursor];
        let t = s.trim_end();
        let w = t.rfind(|c: char| !c.is_alphanumeric() && c != '_').map(|i| i + 1).unwrap_or(0);
        self.cursor = if w == t.len() && !t.is_empty() { prev_boundary(&self.text, t.len()) } else { w };
    }

    fn word_right(&mut self) {
        let s = &self.text[self.cursor..];
        let lead = s.len() - s.trim_start().len();
        let rest = &s[lead..];
        let w = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len()).max(if rest.is_empty() {
            0
        } else {
            1
        });
        self.cursor += lead + w;
        while !self.text.is_char_boundary(self.cursor) {
            self.cursor += 1;
        }
    }

    /// Display width up to the cursor.
    pub fn cursor_width(&self) -> usize {
        str_width(&self.text[..self.cursor])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PromptKind {
    SaveAs,
    OpenFile,
    GotoLine,
    Find,
    Replace,
    ReplaceWith(String),
    VimCmd,
    VimSearch(bool),
    ConfirmQuit,
    ConfirmClose(usize),
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    ConfirmDelete(PathBuf),
    /// Save over a file that changed on disk since it was opened (tab index).
    ConfirmOverwrite(usize),
    /// Interactive replace: y = replace this match, n = skip, a = all, q = stop.
    ReplaceStep {
        find: String,
        rep: String,
    },
}

pub struct Prompt {
    pub kind: PromptKind,
    pub label: String,
    pub input: LineInput,
    /// Cursor/scroll to restore when an incremental search is cancelled.
    pub restore: Option<(crate::text::Pos, Option<crate::text::Pos>, usize)>,
    /// The initial text is a suggestion (the selection, the last search): the first typed
    /// character replaces it, as in VS Code's find box.
    pub fresh: bool,
}

impl Prompt {
    pub fn new(kind: PromptKind, label: impl Into<String>, initial: &str) -> Prompt {
        Prompt { kind, label: label.into(), input: LineInput::with(initial), restore: None, fresh: false }
    }

    /// Yes/no prompts answer on a single key.
    pub fn is_confirm(&self) -> bool {
        matches!(
            self.kind,
            PromptKind::ConfirmQuit
                | PromptKind::ConfirmClose(_)
                | PromptKind::ConfirmDelete(_)
                | PromptKind::ConfirmOverwrite(_)
                | PromptKind::ReplaceStep { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmx_tui::Mods;

    #[test]
    fn editing() {
        let mut l = LineInput::default();
        for c in "hello world".chars() {
            l.handle(&KeyEvent::plain(KeyCode::Char(c)));
        }
        l.handle(&KeyEvent::new(KeyCode::Char('w'), Mods::CTRL));
        assert_eq!(l.text, "hello ");
        l.handle(&KeyEvent::plain(KeyCode::Home));
        l.handle(&KeyEvent::plain(KeyCode::Delete));
        assert_eq!(l.text, "ello ");
        assert_eq!(l.handle(&KeyEvent::plain(KeyCode::Enter)), InputResult::Submit);
    }
}
