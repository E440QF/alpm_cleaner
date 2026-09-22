use crate::db::Pkg;
use crate::privilege::Privilege;
use crate::resolve::{Preview, PreviewEngine};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Name,
    Size,
}

impl SortMode {
    pub fn toggle(&mut self) {
        *self = match self {
            SortMode::Name => SortMode::Size,
            SortMode::Size => SortMode::Name,
        };
    }

    pub fn label(&self) -> &'static str {
        match self {
            SortMode::Name => "name",
            SortMode::Size => "size↓",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceFilter {
    #[default]
    All,
    Repo,
    Aur,
}

impl SourceFilter {
    pub fn cycle(&mut self) {
        *self = match self {
            SourceFilter::All => SourceFilter::Repo,
            SourceFilter::Repo => SourceFilter::Aur,
            SourceFilter::Aur => SourceFilter::All,
        };
    }

    pub fn label(&self) -> &'static str {
        match self {
            SourceFilter::All => "all",
            SourceFilter::Repo => "repo",
            SourceFilter::Aur => "AUR",
        }
    }

    pub fn matches(&self, p: &Pkg) -> bool {
        match self {
            SourceFilter::All => true,
            SourceFilter::Repo => !p.foreign,
            SourceFilter::Aur => p.foreign,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReasonFilter {
    #[default]
    All,
    Explicit,
    Dep,
}

impl ReasonFilter {
    pub fn cycle(&mut self) {
        *self = match self {
            ReasonFilter::All => ReasonFilter::Explicit,
            ReasonFilter::Explicit => ReasonFilter::Dep,
            ReasonFilter::Dep => ReasonFilter::All,
        };
    }

    pub fn label(&self) -> &'static str {
        match self {
            ReasonFilter::All => "all",
            ReasonFilter::Explicit => "expl",
            ReasonFilter::Dep => "dep",
        }
    }

    pub fn matches(&self, p: &Pkg) -> bool {
        match self {
            ReasonFilter::All => true,
            ReasonFilter::Explicit => !p.as_dep,
            ReasonFilter::Dep => p.as_dep,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Filter,
    Confirm,
    Help,
}

pub struct App {
    pub pkgs: Vec<Pkg>,
    pub idx: HashMap<String, usize>,
    pub priv_: Privilege,
    pub checked: HashSet<String>,
    pub cascade: bool,
    pub filter: String,
    pub sort: SortMode,
    pub source: SourceFilter,
    pub reason: ReasonFilter,
    pub mode: Mode,
    pub focus: Focus,
    /// Index into `visible()` for the list cursor.
    pub cursor: usize,
    /// Cursor into `preview_entries()` when the preview pane is focused.
    pub preview_cursor: usize,
    /// Scroll offset for the preview pane; auto-follows `preview_cursor`.
    pub preview_scroll: usize,
    pub preview: Preview,
    /// Generation of the preview request currently wanted.
    pub preview_gen: u64,
    pub preview_pending: bool,
    pub engine: PreviewEngine,
    pub status: String,
}

impl App {
    pub fn new(pkgs: Vec<Pkg>, idx: HashMap<String, usize>, priv_: Privilege) -> Self {
        let mut a = Self {
            pkgs,
            idx,
            priv_,
            checked: HashSet::new(),
            cascade: false,
            filter: String::new(),
            sort: SortMode::Name,
            source: SourceFilter::All,
            reason: ReasonFilter::All,
            mode: Mode::Normal,
            focus: Focus::List,
            cursor: 0,
            preview_cursor: 0,
            preview_scroll: 0,
            preview: Preview::default(),
            preview_gen: 0,
            preview_pending: false,
            engine: PreviewEngine::spawn(),
            status: String::new(),
        };
        a.recompute();
        a
    }

    pub fn reload(&mut self, pkgs: Vec<Pkg>, idx: HashMap<String, usize>) {
        self.pkgs = pkgs;
        self.idx = idx;
        self.checked.retain(|n| self.idx.contains_key(n));
        self.cursor = 0;
        self.preview_cursor = 0;
        self.preview_scroll = 0;
        self.recompute();
    }

    /// Ask the background worker for an exact `pacman --print` preview.
    /// The old preview stays visible (marked stale) until the result arrives.
    pub fn recompute(&mut self) {
        if self.checked.is_empty() {
            self.preview = Preview::default();
            self.preview_pending = false;
            self.preview_cursor = 0;
            self.preview_scroll = 0;
            return;
        }
        let mut targets: Vec<String> = self
            .checked
            .iter()
            .filter(|n| self.idx.contains_key(*n))
            .cloned()
            .collect();
        targets.sort();
        self.preview_gen = self.engine.request(targets, self.cascade);
        self.preview_pending = true;
        self.preview_scroll = 0;
    }

    /// Ordered removal entries: checked first, then pulled in. Backs the
    /// preview cursor.
    pub fn preview_entries(&self) -> Vec<&str> {
        let mut v = self.preview.selected(&self.checked);
        v.extend(self.preview.pulled(&self.checked));
        v
    }

    /// Package under the cursor on whichever pane has focus (for the info panel).
    pub fn hovered(&self) -> Option<&Pkg> {
        match self.focus {
            Focus::List => self.cursor_pkg().map(|pi| &self.pkgs[pi]),
            Focus::Preview => self
                .preview_entries()
                .get(self.preview_cursor)
                .and_then(|n| self.idx.get(*n))
                .map(|&i| &self.pkgs[i]),
        }
    }

    fn clamp_preview_cursor(&mut self) {
        let n = self.preview_entries().len();
        self.preview_cursor = if n == 0 {
            0
        } else {
            self.preview_cursor.min(n - 1)
        };
    }

    pub fn move_preview_cursor(&mut self, delta: isize) {
        let n = self.preview_entries().len();
        if n == 0 {
            self.preview_cursor = 0;
            return;
        }
        let c = (self.preview_cursor as i64).saturating_add(delta as i64);
        self.preview_cursor = c.clamp(0, n as i64 - 1) as usize;
    }

    /// Pick up a finished worker result, if it matches the wanted generation.
    pub fn poll_preview(&mut self) {
        if let Some(p) = self.engine.poll(self.preview_gen) {
            self.preview = p;
            self.preview_pending = false;
            self.preview_scroll = 0;
            self.clamp_preview_cursor();
        }
    }

    /// Indices into `pkgs` matching the current text/source/reason filters,
    /// ordered by the active sort (size is largest-first).
    pub fn visible(&self) -> Vec<usize> {
        let f = self.filter.to_lowercase();
        let mut v: Vec<usize> = (0..self.pkgs.len())
            .filter(|&i| {
                let p = &self.pkgs[i];
                if !self.source.matches(p) || !self.reason.matches(p) {
                    return false;
                }
                if self.filter.is_empty() {
                    return true;
                }
                p.name.to_lowercase().contains(&f) || p.desc.to_lowercase().contains(&f)
            })
            .collect();
        // `pkgs` is stored sorted by name, so only size needs ordering.
        if self.sort == SortMode::Size {
            v.sort_by(|&a, &b| self.pkgs[b].size.cmp(&self.pkgs[a].size));
        }
        v
    }

    pub fn clamp_cursor(&mut self) {
        let n = self.visible().len();
        self.cursor = if n == 0 {
            0
        } else {
            self.cursor.min(n - 1)
        };
    }

    /// One-line summary for the bottom prompt line.
    pub fn state_summary(&self) -> String {
        format!(
            "sort:{} src:{} show:{} · {}/{} shown · {} checked",
            self.sort.label(),
            self.source.label(),
            self.reason.label(),
            self.visible().len(),
            self.pkgs.len(),
            self.checked.len(),
        )
    }

    pub fn cursor_pkg(&self) -> Option<usize> {
        self.visible().get(self.cursor).copied()
    }

    pub fn toggle_cursor(&mut self) {
        if let Some(pi) = self.cursor_pkg() {
            let name = self.pkgs[pi].name.clone();
            if !self.checked.remove(&name) {
                self.checked.insert(name);
            }
            self.recompute();
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.visible().len();
        if n == 0 {
            self.cursor = 0;
            return;
        }
        let c = self.cursor as isize + delta;
        self.cursor = c.clamp(0, n as isize - 1) as usize;
    }

    pub fn total_removal_size(&self) -> u64 {
        let mut size = 0;
        for n in &self.preview.removed {
            if let Some(&i) = self.idx.get(n) {
                size += self.pkgs[i].size;
            }
        }
        size
    }

    pub fn apply_targets(&self) -> Vec<String> {
        // Only the explicitly checked packages go on the command line;
        // pacman resolves cascade/orphans itself. Sorted for determinism.
        let mut v: Vec<String> = self
            .checked
            .iter()
            .filter(|n| self.idx.contains_key(*n))
            .cloned()
            .collect();
        v.sort();
        v
    }

    pub fn command_preview(&self) -> String {
        let mut parts: Vec<String> = self.priv_.prefix_args();
        let mut argv = vec!["pacman".to_string(), "-Rns".to_string()];
        if self.cascade {
            argv.push("-c".to_string());
        }
        argv.extend(self.apply_targets());
        parts.extend(argv);
        if self.apply_targets().is_empty() {
            parts.push("…".to_string());
        }
        parts.join(" ")
    }
}

pub fn human_size(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < U.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", U[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privilege::Privilege;

    fn pkg(name: &str, size: u64, as_dep: bool, foreign: bool) -> Pkg {
        Pkg {
            name: name.into(),
            version: "1".into(),
            desc: format!("{name} description"),
            size,
            as_dep,
            foreign,
            url: String::new(),
            groups: vec![],
            licenses: vec![],
            depends: vec![],
            optdepends: vec![],
            provides: vec![],
            install_date: None,
        }
    }

    fn app() -> App {
        let pkgs = vec![
            pkg("aaa", 100, false, false),
            pkg("bbb", 300, true, false),
            pkg("ccc", 200, false, true),
        ];
        let idx = pkgs
            .iter()
            .enumerate()
            .map(|(i, p)| (p.name.clone(), i))
            .collect();
        App::new(pkgs, idx, Privilege::None)
    }

    fn names(a: &App) -> Vec<String> {
        a.visible().iter().map(|&i| a.pkgs[i].name.clone()).collect()
    }

    #[test]
    fn sort_size_is_largest_first() {
        let mut a = app();
        assert_eq!(names(&a), vec!["aaa", "bbb", "ccc"]);
        a.sort.toggle();
        assert_eq!(names(&a), vec!["bbb", "ccc", "aaa"]);
    }

    #[test]
    fn source_and_reason_filters() {
        let mut a = app();
        a.source.cycle();
        assert_eq!(names(&a), vec!["aaa", "bbb"]);
        a.source.cycle();
        assert_eq!(names(&a), vec!["ccc"]);
        a.source.cycle();
        a.reason.cycle();
        assert_eq!(names(&a), vec!["aaa", "ccc"]);
        a.reason.cycle();
        assert_eq!(names(&a), vec!["bbb"]);
    }

    #[test]
    fn text_filter_combines_with_source() {
        let mut a = app();
        a.filter = "desc".into();
        assert_eq!(names(&a).len(), 3);
        a.filter = "aaa".into();
        assert_eq!(names(&a), vec!["aaa"]);
    }
}
