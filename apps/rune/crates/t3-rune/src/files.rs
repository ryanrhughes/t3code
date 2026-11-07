//! The Files inspector: the thread's workspace as a tree, loaded a folder at a time with
//! `projects.listEntries` as folders open, and a file's source read with `projects.readFile`.
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rune_kit::syntax::Highlight;
use rune_kit::{FileNode, TreeModel, natural_cmp};

use crate::{App, HANDLE};

/// The id suffix of the "Loading…" row a folder holds until its children arrive; it also makes
/// the folder openable, so opening it is what asks for them.
const PLACEHOLDER: char = '\u{1}';

pub fn is_placeholder(id: &str) -> bool {
    id.ends_with(PLACEHOLDER)
}

pub struct FilesPanel {
    /// The workspace on screen: the thread's worktree, else its project's root.
    pub cwd: Option<String>,
    pub tree: TreeModel<FileNode<()>>,
    /// Loaded folders ("" is the root) and their children as (path, is folder), sorted.
    dirs: HashMap<String, Vec<(String, bool)>>,
    pub ignored: HashSet<String>,
    requested: HashSet<String>,
    pub errors: HashMap<String, String>,
    pub open: Option<OpenFile>,
    /// Bumped when the workspace changes, so answers for the last one are dropped.
    seq: u64,
    file_seq: u64,
}

pub struct OpenFile {
    pub path: String,
    pub loading: bool,
    pub error: Option<String>,
    pub text: Rc<str>,
    /// Byte range of each line in `text`.
    pub lines: Rc<[(usize, usize)]>,
    pub highlights: Rc<[Highlight]>,
    pub truncated: bool,
    pub byte_length: u64,
    /// A Markdown file shows rendered until the user asks for its source.
    pub source: bool,
}

impl OpenFile {
    pub fn is_markdown(&self) -> bool {
        let lower = self.path.to_ascii_lowercase();
        lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx")
    }
}

impl Default for FilesPanel {
    fn default() -> Self {
        FilesPanel {
            cwd: None,
            tree: TreeModel::new().key("files-tree"),
            dirs: HashMap::new(),
            ignored: HashSet::new(),
            requested: HashSet::new(),
            errors: HashMap::new(),
            open: None,
            seq: 0,
            file_seq: 0,
        }
    }
}

impl FilesPanel {
    pub fn root_loaded(&self) -> bool {
        self.dirs.contains_key("")
    }

    /// Folders the user opened, top down.
    fn expanded(&self) -> Vec<String> {
        let mut open = Vec::new();
        let mut stack: Vec<String> = self.tree.children(None).map(str::to_owned).collect();
        while let Some(id) = stack.pop() {
            if self.tree.is_expanded(&id) {
                stack.extend(self.tree.children(Some(&id)).map(str::to_owned));
                open.push(id);
            }
        }
        open
    }

    /// Rebuilds the tree from the loaded folders, keeping what was open and the cursor.
    fn rebuild(&mut self) {
        let open = self.expanded();
        let cursor = self.tree.cursor().map(str::to_owned);
        let mut tree = TreeModel::new().key("files-tree");
        self.insert(&mut tree, None, "");
        for id in open {
            tree.set_expanded(&id, true);
        }
        if let Some(c) = cursor.filter(|c| tree.contains(c)) {
            tree.select(&c);
        }
        self.tree = tree;
    }

    fn insert(&self, tree: &mut TreeModel<FileNode<()>>, parent: Option<&str>, dir: &str) {
        let Some(children) = self.dirs.get(dir) else {
            if let Some(p) = parent {
                tree.insert(
                    Some(p),
                    format!("{p}/{PLACEHOLDER}"),
                    FileNode {
                        name: "Loading…".into(),
                        path: String::new(),
                        dir: false,
                        data: None,
                    },
                );
            }
            return;
        };
        for (path, is_dir) in children {
            let name = path.rsplit('/').next().unwrap_or(path).to_owned();
            tree.insert(
                parent,
                path.clone(),
                FileNode {
                    name,
                    path: path.clone(),
                    dir: *is_dir,
                    data: (!is_dir).then_some(()),
                },
            );
            if *is_dir {
                self.insert(tree, Some(path), path);
            }
        }
    }
}

impl FilesPanel {
    /// Headless scenes: lists `dirs` of `cwd` from the disk instead of the server, opens them,
    /// and shows `file`.
    pub fn scene(&mut self, cwd: &str, dirs: &[&str], file: Option<&str>) {
        self.cwd = Some(cwd.to_owned());
        for dir in std::iter::once("").chain(dirs.iter().copied()) {
            let Ok(read) = std::fs::read_dir(std::path::Path::new(cwd).join(dir)) else {
                continue;
            };
            let mut children: Vec<(String, bool)> = read
                .flatten()
                .map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let path = if dir.is_empty() {
                        name
                    } else {
                        format!("{dir}/{name}")
                    };
                    (path, e.file_type().is_ok_and(|t| t.is_dir()))
                })
                .collect();
            children.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| natural_cmp(&a.0, &b.0)));
            self.dirs.insert(dir.to_owned(), children);
            self.requested.insert(dir.to_owned());
        }
        self.rebuild();
        for dir in dirs {
            self.tree.set_expanded(dir, true);
        }
        let Some(path) = file else { return };
        let text =
            std::fs::read_to_string(std::path::Path::new(cwd).join(path)).unwrap_or_default();
        let lang = path.rsplit_once('.').map(|(_, e)| e.to_owned());
        let highlights = rune_kit::syntax::builtin(lang.as_deref(), &text);
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                lines.push((start, i));
                start = i + 1;
            }
        }
        lines.push((start, text.len()));
        self.tree.select(path);
        self.open = Some(OpenFile {
            path: path.to_owned(),
            loading: false,
            error: None,
            byte_length: text.len() as u64,
            text: text.into(),
            lines: lines.into(),
            highlights: highlights.into(),
            truncated: false,
            source: false,
        });
    }
}

impl App {
    /// The workspace the Files tab browses: the open thread's worktree, else its project's root.
    pub fn workspace_cwd(&self) -> Option<String> {
        if let Some(path) = self
            .thread
            .as_ref()
            .and_then(|t| t.thread.worktree_path.clone())
        {
            return Some(path);
        }
        let project = self
            .selected
            .as_deref()
            .and_then(|id| self.shell().thread(id))
            .map(|t| t.project_id.clone())
            .or(self.draft_project.clone())?;
        Some(self.shell().project(&project)?.workspace_root.clone())
    }

    /// Follows the workspace on screen and asks for every folder that is open but not loaded.
    /// Runs from `poll`, so a folder opened by click or by key loads either way.
    pub fn sync_files(&mut self) {
        // Without a session there is nothing to load (and a headless scene fills the panel).
        if self.session().is_none()
            || !self.diff_pane.is_open()
            || self.inspector != crate::Inspector::Files
        {
            return;
        }
        let cwd = self.workspace_cwd();
        if cwd != self.files.cwd {
            self.files = FilesPanel {
                cwd,
                seq: self.files.seq + 1,
                file_seq: self.files.file_seq,
                ..FilesPanel::default()
            };
        }
        if self.files.cwd.is_none() {
            return;
        }
        let mut want = vec![String::new()];
        want.extend(self.files.expanded());
        for dir in want {
            if !self.files.requested.contains(&dir) {
                self.load_dir(dir);
            }
        }
    }

    fn load_dir(&mut self, dir: String) {
        let (Some(cwd), Some(session), Some(handle)) = (
            self.files.cwd.clone(),
            self.session(),
            HANDLE.get().cloned(),
        ) else {
            return;
        };
        self.files.requested.insert(dir.clone());
        let seq = self.files.seq;
        let mut payload = serde_json::json!({ "cwd": cwd });
        if !dir.is_empty() {
            payload["directoryPath"] = dir.clone().into();
        }
        session.call("projects.listEntries", payload, move |result| {
            handle.update(move |s: &mut App, _| {
                if s.files.seq != seq {
                    return;
                }
                let entries = match result {
                    Ok(v) => v["entries"].as_array().cloned().unwrap_or_default(),
                    Err(e) => {
                        s.files.errors.insert(dir.clone(), e);
                        Vec::new()
                    }
                };
                let mut children = Vec::new();
                for e in &entries {
                    let Some(path) = e["path"].as_str() else {
                        continue;
                    };
                    // The server may answer with deeper paths; a folder shows only its own.
                    if path.rsplit_once('/').map_or("", |(d, _)| d) != dir {
                        continue;
                    }
                    if e["ignored"] == true {
                        s.files.ignored.insert(path.to_owned());
                    }
                    children.push((path.to_owned(), e["kind"] == "directory"));
                }
                children.sort_by(|a, b| {
                    b.1.cmp(&a.1).then_with(|| {
                        let name = |p: &str| p.rsplit('/').next().unwrap_or(p).to_owned();
                        natural_cmp(&name(&a.0), &name(&b.0))
                    })
                });
                s.files.dirs.insert(dir, children);
                s.files.rebuild();
            });
        });
    }

    /// Reads `path` into the source view, highlighting it off the UI thread.
    pub fn open_file(&mut self, path: &str) {
        if is_placeholder(path) || self.files.open.as_ref().is_some_and(|f| f.path == path) {
            return;
        }
        let (Some(cwd), Some(session), Some(handle)) = (
            self.files.cwd.clone(),
            self.session(),
            HANDLE.get().cloned(),
        ) else {
            return;
        };
        self.files.file_seq += 1;
        let seq = self.files.file_seq;
        self.files.open = Some(OpenFile {
            path: path.to_owned(),
            loading: true,
            error: None,
            text: "".into(),
            lines: Rc::new([]),
            highlights: Rc::new([]),
            truncated: false,
            byte_length: 0,
            source: false,
        });
        let lang = path
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .filter(|e| rune_kit::syntax::known(e));
        let payload = serde_json::json!({ "cwd": cwd, "relativePath": path });
        session.call("projects.readFile", payload, move |result| {
            let read = result.map(|v| {
                let text = v["contents"].as_str().unwrap_or_default().to_owned();
                let highlights = rune_kit::syntax::builtin(lang.as_deref(), &text);
                let mut lines = Vec::new();
                let mut start = 0;
                for (i, b) in text.bytes().enumerate() {
                    if b == b'\n' {
                        lines.push((start, i));
                        start = i + 1;
                    }
                }
                if start < text.len() || lines.is_empty() {
                    lines.push((start, text.len()));
                }
                let truncated = v["truncated"] == true;
                let byte_length = v["byteLength"].as_u64().unwrap_or(text.len() as u64);
                (text, lines, highlights, truncated, byte_length)
            });
            handle.update(move |s: &mut App, _| {
                let Some(f) = s.files.open.as_mut().filter(|_| s.files.file_seq == seq) else {
                    return;
                };
                f.loading = false;
                match read {
                    Ok((text, lines, highlights, truncated, byte_length)) => {
                        f.text = text.into();
                        f.lines = lines.into();
                        f.highlights = highlights.into();
                        f.truncated = truncated;
                        f.byte_length = byte_length;
                    }
                    Err(e) => f.error = Some(e),
                }
            });
        });
    }

    pub fn close_file(&mut self) {
        self.files.open = None;
    }
}
