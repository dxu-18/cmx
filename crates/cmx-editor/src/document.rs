//! A document: text + file identity + undo history + highlight cache.

use crate::highlight::{HlCache, Lang};
use crate::history::{Coalesce, Edit, History, Sel};
use crate::text::{LineEnding, Pos, Text};
use std::io;
use std::path::{Path, PathBuf};

pub struct Document {
    pub text: Text,
    pub path: Option<PathBuf>,
    pub line_ending: LineEnding,
    pub history: History,
    saved_version: u64,
    pub lang: Lang,
    pub hl: HlCache,
    /// Number shown in "Untitled-N".
    pub untitled: usize,
    /// Extension appended on save-as when the user gives a bare name (e.g. "stex").
    pub default_ext: Option<&'static str>,
    pub tab_width: usize,
    pub use_tabs: bool,
    /// Bumped on every change (render caches key on it).
    pub revision: u64,
    pub lossy: bool,
    /// When set, every recorded edit is also appended here (multi-cursor position mapping).
    pub log: Option<Vec<Edit>>,
    /// (modified time, length) of the file when last read or written: external changes are
    /// detected by comparing against the file on disk.
    pub disk: Option<(std::time::SystemTime, u64)>,
    /// The file changed on disk while this buffer had unsaved edits.
    pub conflict: bool,
    /// The file was deleted on disk.
    pub deleted: bool,
}

impl Document {
    pub fn untitled(n: usize) -> Document {
        Document {
            text: Text::new(),
            path: None,
            line_ending: LineEnding::Lf,
            history: History::new(),
            saved_version: 0,
            lang: Lang::Plain,
            hl: HlCache::default(),
            untitled: n,
            default_ext: None,
            tab_width: 4,
            use_tabs: false,
            revision: 0,
            lossy: false,
            log: None,
            disk: None,
            conflict: false,
            deleted: false,
        }
    }

    /// Open a file; a missing file yields an empty document bound to that path (created on save).
    pub fn open(path: &Path) -> io::Result<Document> {
        let mut d = Document::untitled(0);
        d.lang = Lang::detect(path);
        d.path = Some(path.to_path_buf());
        match std::fs::read(path) {
            Ok(bytes) => {
                let s = match String::from_utf8(bytes) {
                    Ok(s) => s,
                    Err(e) => {
                        d.lossy = true;
                        String::from_utf8_lossy(e.as_bytes()).into_owned()
                    }
                };
                let (t, le) = Text::parse(&s);
                d.text = t;
                d.line_ending = le;
                d.detect_indent();
                d.disk = stamp(path);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        if d.lang == Lang::Makefile || d.lang == Lang::Go {
            d.use_tabs = true;
        }
        Ok(d)
    }

    fn detect_indent(&mut self) {
        let mut tabs = 0;
        let mut spaces = [0usize; 9];
        for l in self.text.lines().iter().take(2000) {
            if l.starts_with('\t') {
                tabs += 1;
            } else if l.starts_with(' ') {
                let n = l.len() - l.trim_start_matches(' ').len();
                if (2..=8).contains(&n) {
                    spaces[n] += 1;
                }
            }
        }
        let sp: usize = spaces.iter().sum();
        if tabs > sp {
            self.use_tabs = true;
        } else if sp > 0 {
            // smallest indent width that explains most lines
            if spaces[2] * 2 >= sp / 2 && spaces[2] > 0 {
                self.tab_width = 2;
            } else if spaces[4] + spaces[8] > 0 {
                self.tab_width = 4;
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.history.version() != self.saved_version
    }

    pub fn display_name(&self) -> String {
        match &self.path {
            Some(p) => {
                p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
            }
            None => format!("Untitled-{}", self.untitled),
        }
    }

    pub fn save(&mut self) -> io::Result<()> {
        let path = self.path.clone().ok_or_else(|| io::Error::other("no file name"))?;
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
            && !dir.exists()
        {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, self.text.to_string_with(self.line_ending))?;
        self.disk = stamp(&path);
        self.conflict = false;
        self.deleted = false;
        self.saved_version = self.history.version();
        self.history.seal();
        self.lossy = false;
        Ok(())
    }

    pub fn save_as(&mut self, path: PathBuf) -> io::Result<()> {
        let path = match (self.default_ext, path.extension()) {
            (Some(ext), None) => path.with_extension(ext),
            _ => path,
        };
        let lang = Lang::detect(&path);
        self.path = Some(path);
        if lang != Lang::Plain || self.lang == Lang::Plain {
            self.lang = lang;
            self.hl.invalidate_from(0);
        }
        self.save()
    }

    // ---- recorded edits (always inside begin/end) ----

    pub fn begin(&mut self, sel: Sel, kind: Coalesce, ch: Option<char>) {
        self.history.begin(sel, kind, ch);
    }

    pub fn end(&mut self, sel: Sel) {
        self.history.end(sel);
    }

    pub fn insert(&mut self, at: Pos, s: &str) -> Pos {
        if s.is_empty() {
            return at;
        }
        let at = self.text.clamp(at);
        let end = self.text.insert(at, s);
        if let Some(log) = &mut self.log {
            log.push(Edit::Insert { at, text: s.to_string() });
        }
        self.history.record(Edit::Insert { at, text: s.to_string() });
        self.hl.invalidate_from(at.line);
        self.revision += 1;
        end
    }

    pub fn remove(&mut self, a: Pos, b: Pos) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let a = self.text.clamp(a);
        let removed = self.text.remove(a, b);
        if !removed.is_empty() {
            if let Some(log) = &mut self.log {
                log.push(Edit::Remove { at: a, text: removed.clone() });
            }
            self.history.record(Edit::Remove { at: a, text: removed.clone() });
            self.hl.invalidate_from(a.line);
            self.revision += 1;
        }
        removed
    }

    pub fn undo(&mut self) -> Option<Sel> {
        let r = self.history.undo(&mut self.text);
        if r.is_some() {
            self.hl.invalidate_from(self.history.last_touched_line());
            self.revision += 1;
        }
        r
    }

    pub fn redo(&mut self) -> Option<Sel> {
        let r = self.history.redo(&mut self.text);
        if r.is_some() {
            self.hl.invalidate_from(self.history.last_touched_line());
            self.revision += 1;
        }
        r
    }

    /// Re-read the file after an external change. The reload is one undoable edit (Ctrl+Z gets
    /// the previous contents back) and leaves the buffer clean. Returns false if nothing changed.
    pub fn reload(&mut self) -> io::Result<bool> {
        let path = self.path.clone().ok_or_else(|| io::Error::other("no file name"))?;
        let bytes = std::fs::read(&path)?;
        let s = String::from_utf8_lossy(&bytes);
        let (t, le) = Text::parse(&s);
        self.disk = stamp(&path);
        self.conflict = false;
        self.deleted = false;
        self.line_ending = le;
        if t.lines() == self.text.lines() {
            self.saved_version = self.history.version();
            return Ok(false);
        }
        let new = t.to_string_with(LineEnding::Lf);
        let sel = (Pos::new(0, 0), None);
        self.history.seal();
        self.begin(sel, Coalesce::None, None);
        let end = self.text.end();
        self.remove(Pos::new(0, 0), end);
        self.insert(Pos::new(0, 0), &new);
        self.end(sel);
        self.history.seal();
        self.saved_version = self.history.version();
        Ok(true)
    }

    /// Prose files (Markdown, LaTeX, stex, .txt) open with soft wrap on.
    pub fn is_prose(&self) -> bool {
        matches!(self.lang, Lang::Markdown | Lang::Latex | Lang::Stex)
            || self.path.as_ref().and_then(|p| p.extension()).is_some_and(|e| e == "txt")
    }

    pub fn indent_unit(&self) -> String {
        if self.use_tabs { "\t".to_string() } else { " ".repeat(self.tab_width) }
    }
}

/// (modified time, length) of a file, for change detection.
pub fn stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}
