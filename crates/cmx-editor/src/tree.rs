//! File explorer: a lazily-expanded directory tree flattened into visible rows.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

pub struct Tree {
    pub root: PathBuf,
    pub nodes: Vec<Node>,
    expanded: HashSet<PathBuf>,
    pub selected: usize,
    pub scroll: usize,
    pub show_hidden: bool,
    /// Set by `reveal`; the renderer scrolls the selection into view.
    pub pending_reveal: bool,
    /// Modification time of the root and every expanded folder at the last refresh: a folder's
    /// mtime changes when entries are created, deleted or renamed in it.
    stamps: Vec<(PathBuf, Option<std::time::SystemTime>)>,
}

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

const HIDDEN: &[&str] = &[".git", ".DS_Store", ".hg", ".svn"];

fn read_sorted(dir: &Path, show_hidden: bool) -> Vec<(PathBuf, String, bool)> {
    let mut v: Vec<(PathBuf, String, bool)> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if HIDDEN.contains(&name.as_str()) || (!show_hidden && name.starts_with('.') && name != ".gitignore") {
                    return None;
                }
                let path = e.path();
                // follow symlinks for the dir check
                let is_dir = std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
                Some((path, name, is_dir))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    v
}

impl Tree {
    pub fn new(root: PathBuf) -> Tree {
        let mut t = Tree {
            root,
            nodes: Vec::new(),
            expanded: HashSet::new(),
            selected: 0,
            scroll: 0,
            show_hidden: true,
            pending_reveal: false,
            stamps: Vec::new(),
        };
        t.refresh();
        t
    }

    pub fn root_name(&self) -> String {
        self.root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    /// Rebuild the visible list from the expanded set (keeps the selection on the same path).
    pub fn refresh(&mut self) {
        let sel_path = self.nodes.get(self.selected).map(|n| n.path.clone());
        self.nodes.clear();
        self.stamps.clear();
        let root = self.root.clone();
        self.walk(&root, 0);
        if let Some(p) = sel_path
            && let Some(i) = self.nodes.iter().position(|n| n.path == p)
        {
            self.selected = i;
        }
        self.selected = self.selected.min(self.nodes.len().saturating_sub(1));
    }

    /// Did any listed folder change on disk since the last refresh?
    pub fn changed_on_disk(&self) -> bool {
        self.stamps.iter().any(|(p, t)| mtime(p) != *t)
    }

    fn walk(&mut self, dir: &Path, depth: usize) {
        self.stamps.push((dir.to_path_buf(), mtime(dir)));
        for (path, name, is_dir) in read_sorted(dir, self.show_hidden) {
            let expanded = is_dir && self.expanded.contains(&path);
            self.nodes.push(Node { path: path.clone(), name, depth, is_dir, expanded });
            if expanded && depth < 64 {
                self.walk(&path, depth + 1);
            }
        }
    }

    pub fn toggle(&mut self, i: usize) {
        let Some(n) = self.nodes.get(i) else { return };
        if !n.is_dir {
            return;
        }
        let p = n.path.clone();
        if !self.expanded.remove(&p) {
            self.expanded.insert(p);
        }
        self.selected = i;
        self.refresh();
    }

    pub fn expand(&mut self, i: usize) {
        if let Some(n) = self.nodes.get(i)
            && n.is_dir
            && !n.expanded
        {
            self.toggle(i);
        }
    }

    pub fn collapse_or_parent(&mut self) {
        let Some(n) = self.nodes.get(self.selected) else { return };
        if n.is_dir && n.expanded {
            let i = self.selected;
            self.toggle(i);
            return;
        }
        let depth = n.depth;
        if depth == 0 {
            return;
        }
        if let Some(p) = (0..self.selected).rev().find(|&j| self.nodes[j].depth < depth) {
            self.selected = p;
        }
    }

    /// Expand parents of `path` and select it.
    pub fn reveal(&mut self, path: &Path) {
        let Ok(rel) = path.strip_prefix(&self.root) else { return };
        let mut cur = self.root.clone();
        let comps: Vec<_> = rel.components().collect();
        for c in comps.iter().take(comps.len().saturating_sub(1)) {
            cur.push(c);
            self.expanded.insert(cur.clone());
        }
        self.refresh();
        if let Some(i) = self.nodes.iter().position(|n| n.path == path) {
            self.selected = i;
        }
        self.pending_reveal = true;
    }

    pub fn selected_node(&self) -> Option<&Node> {
        self.nodes.get(self.selected)
    }

    /// Directory where "new file" should create things (selected dir, or the selected file's dir).
    pub fn target_dir(&self) -> PathBuf {
        match self.selected_node() {
            Some(n) if n.is_dir => n.path.clone(),
            Some(n) => n.path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| self.root.clone()),
            None => self.root.clone(),
        }
    }

    pub fn move_sel(&mut self, delta: isize) {
        if self.nodes.is_empty() {
            return;
        }
        let n = self.nodes.len() as isize;
        self.selected = (self.selected as isize + delta).clamp(0, n - 1) as usize;
    }

    pub fn ensure_visible(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
        let max = self.nodes.len().saturating_sub(height);
        self.scroll = self.scroll.min(max);
    }
}

/// Walk the workspace for quick-open (skips VCS/build dirs, caps the number of files).
pub fn list_files(root: &Path, limit: usize) -> Vec<String> {
    const SKIP: &[&str] = &[
        ".git",
        "target",
        "node_modules",
        ".hg",
        ".svn",
        "build",
        "dist",
        ".venv",
        "__pycache__",
        ".cache",
        ".DS_Store",
    ];
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if SKIP.contains(&name.as_ref()) {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let path = e.path();
            if ft.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}
