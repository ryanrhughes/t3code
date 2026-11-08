//! T3 Code on Rune: a native client for any T3 server (local, LAN, tailnet or tunnel).
//! `cargo run -p t3-rune` opens the window; `-- --snap out.png --scene <name>` renders headless.
mod actions;
mod attach;
mod files;
mod find;
mod keys;
mod notify;
mod picker;
mod plan;
mod prefs;
mod queue;
mod settings;
mod theme;
mod view;
mod workspace;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

use rune::{AppHandle, AppSpec, Chrome, KeyEvent, TextEdit, Tx};
use rune_kit::shell::{self, PaneState};
use t3_client::pairing::{self, Credential};
use t3_client::protocol::{
    self, ModelSelection, ServerConfig, ServerConfigEvent, ThreadStreamItem,
};
use t3_client::session::{self, ClientEvent, SessionHandle, Status};
use t3_client::store::{ShellState, ThreadState};
use t3_theme::ThemeDefinition;

static HANDLE: OnceLock<AppHandle<App>> = OnceLock::new();

pub enum Screen {
    /// No saved pairing: connect to this machine's server, or paste a link.
    Pair {
        link: TextEdit,
        error: Option<String>,
        busy: bool,
        /// A server running on this machine for this user, connectable without a link.
        local: Option<url::Url>,
    },
    Main,
}

impl Screen {
    fn pairing(error: Option<String>) -> Screen {
        let local = pairing::local_home().and_then(|h| pairing::local_server(&h));
        Screen::Pair {
            link: focused(""),
            error,
            busy: false,
            local,
        }
    }
}

/// Which popup is open; one at a time.
#[derive(Clone, Copy, PartialEq)]
pub enum Popup {
    Model,
    Theme,
    Project,
    DiffTurn,
    SettingsTheme,
    SettingsAccess,
    SettingsMachine,
    BaseBranch,
}

/// Which changes the diff panel shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiffSel {
    /// The newest turn's changes, following new turns as they land.
    Latest,
    /// One turn's changes, by its checkpoint turn count.
    Turn(u64),
    /// Everything since the thread began.
    All,
}

/// What the inspector beside the thread shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Inspector {
    #[default]
    Changes,
    Files,
}

pub struct DiffPanel {
    pub sel: DiffSel,
    pub state: rune_kit::diff::DiffState,
    pub loading: bool,
    pub error: Option<String>,
    /// The (thread, from, to) range on screen, so a reload only happens when it changes.
    loaded: Option<(String, u64, u64)>,
    seq: u64,
}

impl Default for DiffPanel {
    fn default() -> Self {
        DiffPanel {
            sel: DiffSel::Latest,
            state: rune_kit::diff::DiffState::new(Vec::new()),
            loading: false,
            error: None,
            loaded: None,
            seq: 0,
        }
    }
}

/// One paired environment: a T3 server on this machine or another, with its own socket.
pub struct Env {
    pub cred: Credential,
    pub session: Option<Rc<SessionHandle>>,
    pub status: Status,
    pub config: Option<ServerConfig>,
    pub shell: ShellState,
    /// Theme files on this machine (Omarchy's); the theme environment's are the ones in use.
    pub themes: Vec<ThemeDefinition>,
    /// Which session the events come from; a replaced session's late ones are dropped.
    generation: u64,
    /// The server's resolved keybindings: the web's defaults plus the user's.
    pub keys: Vec<t3_client::keybindings::Rule>,
    pub watch: notify::Watch,
}

impl Env {
    /// The machine's name: the server's own label once connected, else the paired one.
    pub fn label(&self) -> &str {
        self.config
            .as_ref()
            .map(|c| c.environment.label.as_str())
            .filter(|l| !l.is_empty())
            .unwrap_or(&self.cred.label)
    }

    /// A server on this machine, whose theme files (Omarchy's) the client follows.
    fn is_local(&self) -> bool {
        url::Url::parse(&self.cred.http_base).is_ok_and(|u| {
            matches!(
                u.host_str(),
                Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
            )
        })
    }
}

static NO_SHELL: std::sync::LazyLock<ShellState> = std::sync::LazyLock::new(ShellState::default);

pub struct App {
    pub screen: Screen,
    /// Every paired environment, merged in the sidebar.
    pub envs: Vec<Env>,
    /// The environment of the open thread or draft; sends and lookups go to it.
    pub focus: usize,
    pub env_themes: Vec<ThemeDefinition>,
    pub prefs: prefs::Prefs,
    pub looks: theme::Looks,
    pub sidebar: PaneState,
    /// The inspector beside the thread, showing the changes or the files.
    pub diff_pane: PaneState,
    pub inspector: Inspector,
    /// The settings page, in place of the thread.
    pub settings_open: bool,
    /// The machine whose server settings the page shows, by environment id.
    pub settings_env: Option<String>,
    /// Settings → Machines: a pairing link for another machine.
    pub add_link: TextEdit,
    pub adding: bool,
    pub add_error: Option<String>,
    pub diff: DiffPanel,
    pub files: files::FilesPanel,
    pub collapsed: HashSet<String>,
    /// The open thread, or a draft in `draft_project` when None.
    pub selected: Option<String>,
    pub draft_project: Option<String>,
    pub thread: Option<ThreadState>,
    pub composer: TextEdit,
    /// The model the next send uses; None follows the thread's.
    pub model: Option<ModelSelection>,
    pub popup: Option<Popup>,
    /// The suggestion menu the composer's caret opened (`@` files, `/` commands, `$` skills).
    pub menu: Option<ComposerMenu>,
    /// The command palette (Ctrl/Cmd+K), when open.
    pub palette: Option<CommandPalette>,
    /// A sidebar thread's context menu.
    pub thread_menu: Option<actions::ThreadMenu>,
    pub rename: Option<actions::Rename>,
    /// Plan mode for the next new thread (an open thread keeps its own on the server).
    pub draft_plan: bool,
    /// The Ctrl/Cmd+P file picker, when open.
    pub picker: Option<picker::FilePicker>,
    /// Find in the open thread (Ctrl/Cmd+F), when open.
    pub find: Option<find::Find>,
    /// A new thread's workspace: a new worktree (Some(true)), the checkout, or the server default.
    pub draft_worktree: Option<bool>,
    /// A picked base branch, with the machine and project root it was picked for.
    pub draft_base: Option<(String, String, String)>,
    pub refs: Option<workspace::Refs>,
    /// The queued message the composer is rewriting, by run id.
    pub editing_queued: Option<String>,
    /// The ordinary draft, set aside while the composer edits a queued message.
    pub stashed_draft: Option<TextEdit>,
    /// Files on their way into the next message.
    pub attachments: Vec<attach::Attachment>,
    next_attachment: u64,
    /// Settings → Archived threads, once loaded.
    pub archived: Option<settings::Archived>,
    /// The open thread's read watermark, kept on the server for every client.
    pub visit: Option<actions::Visit>,
    /// When the soonest snoozed thread wakes, so the sidebar moves it back on time.
    snooze_wake: Option<i64>,
    menu_seq: u64,
    /// Selected options per pending question: request id → question id → values.
    pub answers: HashMap<String, HashMap<String, Vec<String>>>,
    /// (id, message, wall-clock second it expires at).
    pub toasts: Vec<(u64, String, i64)>,
    next_toast: u64,
    /// Reasoning rows the user opened.
    pub expanded: HashSet<String>,
    /// Bumped whenever the looks are rebuilt, so memoized rows repaint on a theme change.
    pub theme_generation: u64,
    /// Wall-clock seconds, advanced once a second only while something works, so elapsed
    /// labels update without any continuous animation.
    pub clock: i64,
    /// Below `NARROW` points the sidebar folds away, as a tiled window is often that narrow;
    /// `sidebar_before_narrow` restores the user's choice when the window widens again.
    pub narrow: bool,
    sidebar_before_narrow: Option<bool>,
    window_width: f32,
    /// The inspector width the user chose, while `fit` holds it narrower.
    inspector_wanted: Option<f32>,
    /// The width `fit` last gave the inspector, to tell its own changes from the user's.
    inspector_fitted: Option<f32>,
    next_generation: u64,
    /// Notifications only go out while the window is in the background.
    pub window_focused: bool,
    /// Output of expanded command rows, by turn item id. Only expanded rows hold a stream.
    pub outputs: HashMap<String, CommandOutput>,
}

/// One expanded command row's output.
pub struct CommandOutput {
    pub out: t3_client::terminal::TerminalOutput,
    /// Still receiving; false once the command settled or the fetch answered.
    pub running: bool,
    pub loaded: bool,
    pub error: Option<String>,
}

pub struct CommandPalette {
    pub query: TextEdit,
    pub active: usize,
}

#[derive(Clone)]
pub enum PaletteAction {
    NewThread,
    ToggleDiff,
    ToggleFiles,
    Settings,
    ToggleSidebar,
    OpenThread(usize, String),
    Theme(String),
}

pub struct PaletteItem {
    pub group: &'static str,
    pub label: String,
    pub detail: Option<String>,
    pub shortcut: Option<&'static str>,
    pub action: PaletteAction,
}

/// How well `label` matches `query`, lower is better: whole-label prefix, then a word's start,
/// then anywhere. `None` when it doesn't match.
fn palette_rank(label: &str, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }
    let (l, q) = (label.to_lowercase(), query.to_lowercase());
    if l.starts_with(&q) {
        Some(0)
    } else if l
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(&q))
    {
        Some(1)
    } else {
        l.contains(&q).then_some(2)
    }
}

pub struct ComposerMenu {
    pub trigger: t3_client::composer::Trigger,
    pub items: Vec<MenuItem>,
    pub active: usize,
    /// Waiting on a file search.
    pub loading: bool,
}

pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    pub action: MenuAction,
}

pub enum MenuAction {
    /// Replaces the trigger with this text: a file link, `/command ` or `$skill `.
    Insert(String),
    /// `/model`: clears the trigger and opens the model picker.
    OpenModel,
}

/// Items whose label matches `query`: prefix matches first, then anywhere, case-insensitively.
fn filter_items(items: Vec<MenuItem>, query: &str) -> Vec<MenuItem> {
    let q = query.to_lowercase();
    let (mut prefix, mut inner): (Vec<_>, Vec<_>) = items
        .into_iter()
        .filter(|i| i.label.to_lowercase().contains(&q))
        .partition(|i| {
            i.label
                .to_lowercase()
                .trim_start_matches(['/', '$'])
                .starts_with(&q)
        });
    prefix.append(&mut inner);
    prefix
}

/// The session subscription key for an item's output stream.
fn output_key(item_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("command-output", item_id).hash(&mut h);
    h.finish()
}

/// `ATTACHMENT_ONLY_BOOTSTRAP_PROMPT`: what a message of only attachments says.
const ATTACHMENT_ONLY_PROMPT: &str = "[User attached one or more files without additional text. Respond using the conversation context and the attached files.]";

const NARROW: f32 = 720.0;
/// The thread column never gets narrower than this while the window can afford it.
const MAIN_MIN: f32 = 420.0;

impl App {
    fn new() -> App {
        let prefs = prefs::Prefs::load();
        let looks = theme::Looks::build(&prefs.theme(), &[]);
        let creds = pairing::load_credentials();
        let mut app = App {
            screen: if creds.is_empty() {
                Screen::pairing(None)
            } else {
                Screen::Main
            },
            envs: Vec::new(),
            focus: 0,
            env_themes: Vec::new(),
            prefs,
            looks,
            sidebar: PaneState::new(276.0).range(200.0, 520.0),
            diff_pane: PaneState::new(620.0).range(380.0, 1200.0).collapsed(),
            inspector: Inspector::Changes,
            settings_open: false,
            settings_env: None,
            add_link: TextEdit::new(""),
            adding: false,
            add_error: None,
            diff: DiffPanel::default(),
            files: files::FilesPanel::default(),
            collapsed: HashSet::new(),
            selected: None,
            draft_project: None,
            thread: None,
            composer: focused(""),
            model: None,
            popup: None,
            answers: HashMap::new(),
            toasts: Vec::new(),
            next_toast: 0,
            expanded: HashSet::new(),
            theme_generation: 0,
            clock: now_secs(),
            narrow: false,
            sidebar_before_narrow: None,
            window_width: 0.0,
            inspector_wanted: None,
            inspector_fitted: None,
            next_generation: 0,
            window_focused: true,
            outputs: HashMap::new(),
            menu: None,
            menu_seq: 0,
            palette: None,
            thread_menu: None,
            rename: None,
            draft_plan: false,
            picker: None,
            find: None,
            draft_worktree: None,
            draft_base: None,
            refs: None,
            editing_queued: None,
            stashed_draft: None,
            attachments: Vec::new(),
            next_attachment: 0,
            archived: None,
            visit: None,
            snooze_wake: None,
        };
        // A scene draws captured data; a live session would replace it.
        let scene = std::env::args().any(|a| a == "--scene");
        if creds.is_empty() {
            // First launch on a machine that runs T3 Code: connect to it with no link. A revoked
            // or forgotten pairing lands on the pairing screen instead, so revoking sticks.
            if matches!(app.screen, Screen::Pair { local: Some(_), .. }) && !headless() {
                app.connect_local()
            }
        } else if !scene {
            for c in creds {
                app.start(c);
            }
        }
        app
    }

    /// Connects in the background. Events land through `AppHandle::update`, so nothing polls.
    /// Pairing an environment again replaces its session.
    fn start(&mut self, cred: Credential) {
        let env_id = cred.environment_id.clone();
        self.next_generation += 1;
        let generation = self.next_generation;
        let session = HANDLE.get().cloned().map(|handle| {
            let id = env_id.clone();
            Rc::new(session::spawn(
                cred.clone(),
                std::sync::Arc::new(move |ev| {
                    let id = id.clone();
                    handle.update(move |s, tx| s.on_event(&id, generation, ev, tx))
                }),
            ))
        });
        let env = Env {
            cred,
            session,
            status: Status::Connecting,
            config: None,
            shell: ShellState::default(),
            themes: Vec::new(),
            generation,
            watch: Default::default(),
            keys: Vec::new(),
        };
        match self.env_index(&env_id) {
            Some(i) => {
                self.envs[i] = env;
                // Pairing the open thread's machine again: the new session watches it too.
                if i == self.focus {
                    if let Some(s) = self.session() {
                        s.watch_thread(self.selected.clone());
                    }
                }
            }
            None => self.envs.push(env),
        }
    }

    /// A machine's name, with its address when another paired machine has the same name.
    pub fn env_name(&self, idx: usize) -> String {
        let Some(e) = self.envs.get(idx) else {
            return String::new();
        };
        let label = e.label();
        let shared = self
            .envs
            .iter()
            .enumerate()
            .any(|(j, o)| j != idx && o.label() == label);
        if !shared {
            return label.to_owned();
        }
        let host = url::Url::parse(&e.cred.http_base)
            .ok()
            .and_then(|u| Some(format!("{}:{}", u.host_str()?, u.port_or_known_default()?)))
            .unwrap_or_else(|| e.cred.http_base.clone());
        format!("{label} ({host})")
    }

    pub fn env_index(&self, environment_id: &str) -> Option<usize> {
        self.envs
            .iter()
            .position(|e| e.cred.environment_id == environment_id)
    }

    pub fn env(&self) -> Option<&Env> {
        self.envs.get(self.focus)
    }

    /// The focused environment's session; an `Rc`, so holding it doesn't borrow the app.
    pub fn session(&self) -> Option<Rc<SessionHandle>> {
        self.env()?.session.clone()
    }

    pub fn config(&self) -> Option<&ServerConfig> {
        self.env()?.config.as_ref()
    }

    pub fn cred(&self) -> Option<&Credential> {
        self.env().map(|e| &e.cred)
    }

    pub fn status(&self) -> Status {
        self.env().map_or(Status::Connecting, |e| e.status.clone())
    }

    /// The focused environment's projects and threads.
    pub fn shell(&self) -> &ShellState {
        self.env().map_or(&NO_SHELL, |e| &e.shell)
    }

    /// The environment whose theme files and `t3 theme set` default the client follows: the
    /// one on this machine, else the first paired.
    fn theme_env(&self) -> usize {
        self.envs.iter().position(Env::is_local).unwrap_or(0)
    }

    fn rebuild_looks(&mut self) {
        self.looks = theme::Looks::build(&self.prefs.theme(), &self.env_themes);
        self.theme_generation += 1;
    }

    pub fn toast(&mut self, msg: impl Into<String>, tx: &mut Tx) {
        self.next_toast += 1;
        let id = self.next_toast;
        self.toasts.push((id, msg.into(), now_secs() + 6));
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
        tx.wake_after(std::time::Duration::from_secs(6));
    }

    fn on_event(&mut self, env_id: &str, generation: u64, ev: ClientEvent, tx: &mut Tx) {
        DIAG_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Some(idx) = self
            .env_index(env_id)
            .filter(|&i| self.envs[i].generation == generation)
        else {
            return;
        };
        let themes = idx == self.theme_env();
        match ev {
            ClientEvent::Status(s) => self.envs[idx].status = s,
            ClientEvent::Config(ev) => match ev {
                ServerConfigEvent::Snapshot { config } => {
                    // `t3 theme set <id>` is adopted once per generation, as the web does.
                    if let Some(generation) = config.default_theme().filter(|_| themes) {
                        if self.prefs.adopt_default_theme(&generation) {
                            self.rebuild_looks();
                        }
                    }
                    self.envs[idx].keys = t3_client::keybindings::decode(&config.keybindings);
                    self.envs[idx].config = Some(config);
                }
                ServerConfigEvent::KeybindingsUpdated { payload } => {
                    self.envs[idx].keys = t3_client::keybindings::decode(&payload.keybindings);
                }
                ServerConfigEvent::ProviderStatuses { payload } => {
                    if let Some(c) = self.envs[idx].config.as_mut() {
                        c.providers = payload.providers;
                    }
                }
                ServerConfigEvent::SettingsUpdated { payload } => {
                    let mut adopt = None;
                    if let Some(c) = self.envs[idx].config.as_mut() {
                        c.settings = payload.settings;
                        adopt = c.default_theme().filter(|_| themes);
                    }
                    if let Some(generation) = adopt {
                        if self.prefs.adopt_default_theme(&generation) {
                            self.rebuild_looks();
                        }
                    }
                }
                ServerConfigEvent::EnvironmentThemesUpdated { payload } => {
                    // Omarchy rewrites themes/omarchy.json on every theme switch; this repaints live.
                    self.envs[idx].themes = payload
                        .themes
                        .iter()
                        .filter_map(t3_theme::environment_theme)
                        .collect();
                    if themes {
                        self.env_themes = self.envs[idx].themes.clone();
                        tx.color_motion(rune::Spring::new(0.4, 0.0));
                        self.rebuild_looks();
                    }
                }
                ServerConfigEvent::Unknown => {}
            },
            ClientEvent::Shell(item) => {
                // The first snapshot lands without motion so the sidebar doesn't animate in.
                if !self.envs[idx].shell.loaded {
                    tx.instant();
                }
                self.envs[idx].shell.apply(item);
                self.watch_threads(idx);
                if idx == self.focus {
                    self.track_visit();
                }
                if idx == self.focus && self.selected.is_none() && self.draft_project.is_none() {
                    self.draft_project = self
                        .shell()
                        .sidebar_projects()
                        .first()
                        .map(|p| p.id.clone());
                }
            }
            ClientEvent::Thread { thread_id, item } => {
                if idx != self.focus || self.selected.as_deref() != Some(&thread_id) {
                    return;
                }
                match item {
                    ThreadStreamItem::Snapshot {
                        sequence,
                        projection,
                    } => {
                        // Opening a thread lands at its end with nothing animating in.
                        tx.settle();
                        self.thread = Some(ThreadState::from_snapshot(sequence, *projection));
                        self.sync_diff();
                    }
                    ThreadStreamItem::Event {
                        sequence,
                        thread_id,
                        event,
                    } => {
                        let checkpoint =
                            matches!(event, t3_client::protocol::ThreadEvent::Checkpoint(_));
                        if let Some(t) = self.thread.as_mut() {
                            t.apply(sequence, &thread_id, event);
                        }
                        // A finished turn moves "latest"; the diff panel follows it.
                        if checkpoint {
                            self.sync_diff();
                        }
                    }
                    ThreadStreamItem::Synchronized => {
                        if let Some(t) = self.thread.as_mut() {
                            t.synchronized = true;
                        }
                    }
                    ThreadStreamItem::Unknown => {}
                }
            }
            ClientEvent::ThreadLaunched { thread_id } => self.open_thread_in(idx, thread_id),
            ClientEvent::CommandFailed(msg) => {
                let msg = if self.envs.len() > 1 {
                    format!("{}: {msg}", self.env_name(idx))
                } else {
                    msg
                };
                self.toast(msg, tx)
            }
            ClientEvent::Unauthorized(msg) => {
                let label = self.env_name(idx);
                self.remove_env(idx, tx);
                if self.envs.is_empty() {
                    self.screen = Screen::pairing(Some(format!("Pair again: {msg}")));
                } else {
                    self.toast(format!("{label} revoked this client: {msg}"), tx);
                }
            }
        }
    }

    /// Forgets an environment: its credential, its socket and anything open from it.
    pub fn remove_env(&mut self, idx: usize, tx: &mut Tx) {
        if idx >= self.envs.len() {
            return;
        }
        if let Err(e) = pairing::forget_credential(&self.envs[idx].cred.environment_id) {
            self.toast(format!("Couldn't save the change: {e}"), tx);
        }
        let theme_owner = self
            .envs
            .get(self.theme_env())
            .map(|e| e.cred.environment_id.clone());
        if idx == self.focus {
            self.clear_thread_view();
            self.selected = None;
        }
        self.envs.remove(idx);
        if idx == self.focus {
            self.focus = 0;
            self.draft_project = self
                .shell()
                .sidebar_projects()
                .first()
                .map(|p| p.id.clone());
        } else if idx < self.focus {
            self.focus -= 1;
        }
        // The machine whose themes the client follows may have changed.
        let owner = self.envs.get(self.theme_env());
        if owner.map(|e| &e.cred.environment_id) != theme_owner.as_ref() {
            self.env_themes = owner.map(|e| e.themes.clone()).unwrap_or_default();
            if let Some(generation) = owner.and_then(|e| e.config.as_ref()?.default_theme()) {
                self.prefs.adopt_default_theme(&generation);
            }
            self.rebuild_looks();
        }
    }

    /// Drops everything shown for the open thread: its detail, streams, diff and model pick.
    /// The diff's request counter keeps climbing, so a late answer can't land in the next view.
    fn clear_thread_view(&mut self) {
        self.find = None;
        if self.editing_queued.take().is_some() {
            self.composer = self.stashed_draft.take().unwrap_or_else(|| focused(""));
        }
        self.thread = None;
        self.model = None;
        self.popup = None;
        self.visit = None;
        self.close_outputs();
        self.diff = DiffPanel {
            sel: self.diff.sel,
            seq: self.diff.seq + 1,
            ..DiffPanel::default()
        };
    }

    /// Opens a thread from any environment, moving the focus there.
    pub fn open_thread_in(&mut self, env: usize, id: String) {
        self.focus_env(env);
        self.open_thread(Some(id));
    }

    fn focus_env(&mut self, env: usize) {
        if env == self.focus || env >= self.envs.len() {
            return;
        }
        if let Some(s) = self.session() {
            s.watch_thread(None);
        }
        self.clear_thread_view();
        self.focus = env;
        self.selected = None;
        self.draft_project = None;
    }

    pub fn open_thread(&mut self, id: Option<String>) {
        if self.selected == id {
            return;
        }
        self.clear_thread_view();
        if let Some(t) = id.as_deref().and_then(|id| self.shell().thread(id)) {
            self.draft_project = Some(t.project_id.clone());
        }
        self.selected = id.clone();
        if let Some(s) = self.session() {
            s.watch_thread(id);
        }
        self.begin_visit();
        self.composer.focused = true;
    }

    /// A draft in `project` of environment `env`.
    pub fn new_thread_in(&mut self, env: usize, project_id: String) {
        self.focus_env(env);
        self.new_thread(Some(project_id));
    }

    pub fn new_thread(&mut self, project_id: Option<String>) {
        let project = project_id
            .or_else(|| self.draft_project.clone())
            .or_else(|| {
                self.shell()
                    .sidebar_projects()
                    .first()
                    .map(|p| p.id.clone())
            });
        self.open_thread(None);
        self.draft_project = project;
    }

    /// The selection a send uses: the user's pick, else the thread's, else the project and
    /// server defaults, else the first ready provider's default model.
    pub fn effective_model(&self) -> Option<ModelSelection> {
        if let Some(m) = &self.model {
            return Some(m.clone());
        }
        if let Some(t) = self
            .selected
            .as_deref()
            .and_then(|id| self.shell().thread(id))
        {
            return Some(t.model_selection.clone());
        }
        let config = self.config()?;
        let project_default = self
            .draft_project
            .as_deref()
            .and_then(|p| self.shell().project(p))
            .and_then(|p| p.default_model_selection.clone());
        project_default
            .or_else(|| config.default_model_selection())
            .or_else(|| {
                let ready = config
                    .providers
                    .iter()
                    .find(|p| p.usable() && p.status == "ready")
                    .or_else(|| {
                        config
                            .providers
                            .iter()
                            .find(|p| p.usable() && p.status != "error")
                    })?;
                let model = ready
                    .models
                    .iter()
                    .find(|m| m.is_default == Some(true) && !m.is_custom)
                    .or_else(|| ready.models.iter().find(|m| !m.is_custom))
                    .or(ready.models.first())?;
                Some(ModelSelection {
                    instance_id: ready.instance_id.clone(),
                    model: model.slug.clone(),
                    options: None,
                })
            })
    }

    /// Sends the composer. `alternate` is the Ctrl/Cmd variant: queue instead of steer while the
    /// agent works.
    pub fn send(&mut self, alternate: bool, tx: &mut Tx) {
        let mut text = self.composer.text.trim().to_owned();
        if let Some(run) = self.editing_queued.clone() {
            if !text.is_empty() {
                self.save_queued_edit(run, text, tx);
            }
            return;
        }
        let env_id = self.env().map(|e| e.cred.environment_id.clone());
        let pending = self
            .attachments
            .iter()
            .any(|a| Some(&a.env_id) == env_id.as_ref());
        if text.is_empty() && !pending {
            // Sending nothing while a plan waits implements it, as the web's composer does.
            if self.ready_plan().is_some() {
                self.implement_plan(tx);
            }
            return;
        }
        let Some(session) = self.session() else {
            return;
        };
        let files = match self.ready_attachments() {
            Ok(files) => files,
            Err(why) => {
                self.toast(why, tx);
                return;
            }
        };
        if text.is_empty() {
            // Files alone go with the web's stand-in prompt.
            text = ATTACHMENT_ONLY_PROMPT.to_owned();
        }
        let Some(model) = self.effective_model() else {
            self.toast("No provider is ready on this server", tx);
            return;
        };
        match self.selected.clone() {
            Some(id) => {
                let thread = self.thread.as_ref();
                let active = thread.is_some_and(|t| t.active_run().is_some());
                let approvals_waiting = thread.is_some_and(|t| {
                    t.runtime_requests.iter().any(|r| {
                        r.status == "pending"
                            && !matches!(
                                r.kind.as_str(),
                                "user_input" | "dynamic_tool_call" | "auth_refresh"
                            )
                    })
                });
                let can_steer = thread.is_some_and(|t| t.can_steer());
                let mode = protocol::DispatchMode::resolve(
                    active,
                    alternate,
                    approvals_waiting,
                    can_steer,
                    self.prefs.follow_up == "queue",
                );
                let mut cmd = protocol::message_dispatch(&id, &text, self.model.as_ref(), mode);
                protocol::attach(&mut cmd, &files);
                session.dispatch(cmd);
            }
            None => {
                let Some(project) = self.draft_project.clone() else {
                    self.toast("Pick a project first", tx);
                    return;
                };
                let runtime = self
                    .config()
                    .map_or("full-access".to_owned(), |c| c.default_runtime_mode());
                if self.draft_in_worktree() && self.draft_base().is_none() {
                    self.toast("Waiting for the branch list to pick a base branch", tx);
                    return;
                }
                let mut input = protocol::launch_thread(&project, &text, &model, &runtime);
                input["workspaceStrategy"] = self.draft_workspace();
                protocol::attach(&mut input, &files);
                if self.draft_plan {
                    input["interactionMode"] = "plan".into();
                }
                session.launch(input);
            }
        }
        self.composer = focused("");
        let env = self.env().map(|e| e.cred.environment_id.clone());
        self.attachments.retain(|a| Some(&a.env_id) != env.as_ref());
    }

    pub fn stop(&mut self) {
        let (Some(session), Some(t)) = (self.session(), &self.thread) else {
            return;
        };
        let thread_stop = self
            .config()
            .is_some_and(|c| c.environment.capability("threadStop"));
        if let Some(cmd) = protocol::stop(
            &t.thread.id,
            thread_stop,
            t.interruptible_run().map(|r| r.id.as_str()),
        ) {
            session.dispatch(cmd);
        }
    }

    pub fn respond(&mut self, request_id: &str, decision: &str) {
        let (Some(session), Some(id)) = (self.session(), &self.selected) else {
            return;
        };
        session.dispatch(protocol::respond_decision(id, request_id, decision));
    }

    pub fn answer(&mut self, request_id: &str) {
        let (Some(session), Some(id)) = (self.session(), &self.selected) else {
            return;
        };
        let picks = self.answers.remove(request_id).unwrap_or_default();
        let answers: serde_json::Map<_, _> = picks
            .into_iter()
            .map(|(q, mut v)| {
                (
                    q,
                    if v.len() == 1 {
                        serde_json::Value::String(v.remove(0))
                    } else {
                        serde_json::json!(v)
                    },
                )
            })
            .collect();
        session.dispatch(protocol::respond_answers(
            id,
            request_id,
            serde_json::Value::Object(answers),
        ));
    }

    pub fn set_theme(&mut self, id: &str, tx: &mut Tx) {
        self.prefs.set_theme(id);
        tx.color_motion(rune::Spring::new(0.4, 0.0));
        self.rebuild_looks();
        self.popup = None;
    }

    /// Opens or closes a command row's output: a live stream where the server streams command
    /// output (`commandOutputStreaming`), else the settled output once.
    pub fn toggle_output(&mut self, item_id: &str) {
        if self.outputs.remove(item_id).is_some() {
            if let Some(session) = self.session() {
                session.unsubscribe(output_key(item_id));
            }
            return;
        }
        let (Some(session), Some(thread), Some(handle)) =
            (self.session(), &self.thread, HANDLE.get().cloned())
        else {
            return;
        };
        let empty = CommandOutput {
            out: Default::default(),
            running: true,
            loaded: false,
            error: None,
        };
        self.outputs.insert(item_id.to_owned(), empty);
        let payload = serde_json::json!({ "threadId": thread.thread.id, "itemId": item_id });
        let id = item_id.to_owned();
        let streaming = self
            .config()
            .is_some_and(|c| c.environment.capability("commandOutputStreaming"));
        if streaming {
            session.subscribe(
                output_key(item_id),
                "orchestration.subscribeCommandOutput",
                payload,
                move |frame| {
                    let id = id.clone();
                    handle.update(move |s: &mut App, _| s.on_output_frame(&id, frame));
                },
            );
        } else {
            session.call("orchestration.getTurnItem", payload, move |result| {
                handle.update(move |s: &mut App, _| {
                    let Some(o) = s.outputs.get_mut(&id) else {
                        return;
                    };
                    (o.loaded, o.running) = (true, false);
                    match result {
                        Ok(v) => o.out.append(v["item"]["output"].as_str().unwrap_or("")),
                        Err(e) => o.error = Some(e),
                    }
                });
            });
        }
    }

    fn on_output_frame(&mut self, item_id: &str, frame: Option<Result<serde_json::Value, String>>) {
        let Some(o) = self.outputs.get_mut(item_id) else {
            return;
        };
        match frame {
            Some(Ok(f)) => {
                o.loaded = true;
                o.out.apply_frame(
                    f["kind"] == "replace",
                    f["text"].as_str().unwrap_or(""),
                    f["truncated"] == true,
                );
                o.running = f["running"] == true;
            }
            Some(Err(e)) => (o.error, o.running) = (Some(e), false),
            None => o.running = false,
        }
    }

    fn close_outputs(&mut self) {
        if let Some(session) = self.session() {
            for id in self.outputs.keys() {
                session.unsubscribe(output_key(id));
            }
        }
        self.outputs.clear();
    }

    pub fn toggle_diff(&mut self) {
        self.toggle_inspector(Inspector::Changes);
    }

    /// Shows `tab` in the inspector, or closes the inspector when it already shows it.
    pub fn toggle_inspector(&mut self, tab: Inspector) {
        let open = !(self.diff_pane.is_open() && self.inspector == tab);
        self.inspector = tab;
        self.diff_pane.set_open(open);
        self.fit();
        self.sync_diff();
        self.sync_files();
    }

    pub fn show_inspector(&mut self, tab: Inspector) {
        self.inspector = tab;
        self.popup = None;
        self.diff_pane.set_open(true);
        self.fit();
        self.sync_diff();
        self.sync_files();
    }

    pub fn open_diff(&mut self, sel: DiffSel) {
        self.diff.sel = sel;
        self.show_inspector(Inspector::Changes);
    }

    /// The (from, to) turn counts the selection spans in the open thread, if it has turns.
    pub fn diff_range(&self) -> Option<(u64, u64)> {
        let turns = self.thread.as_ref()?.diff_turns();
        let latest = turns.first()?.app_run_ordinal?;
        Some(match self.diff.sel {
            DiffSel::Latest => (latest.saturating_sub(1), latest),
            DiffSel::Turn(n) => (n.saturating_sub(1), n),
            DiffSel::All => (0, latest),
        })
    }

    /// Loads the selected range when the panel is open and it changed: a new selection, another
    /// thread, or (following the latest turn) a turn that just finished.
    pub fn sync_diff(&mut self) {
        if !self.diff_pane.is_open() || self.inspector != Inspector::Changes {
            return;
        }
        let (Some(thread), Some((from, to))) = (
            self.thread.as_ref().map(|t| t.thread.id.clone()),
            self.diff_range(),
        ) else {
            self.diff.state.set_files(Vec::new());
            self.diff.loaded = None;
            return;
        };
        let want = (thread.clone(), from, to);
        if self.diff.loaded.as_ref() == Some(&want) {
            return;
        }
        let (Some(session), Some(handle)) = (self.session(), HANDLE.get().cloned()) else {
            return;
        };
        self.diff.loaded = Some(want);
        self.diff.loading = true;
        self.diff.error = None;
        self.diff.seq += 1;
        let seq = self.diff.seq;
        // A range from the start is the whole thread's diff; any other is a span of turns.
        let (tag, payload) = if from == 0 {
            (
                "orchestration.getFullThreadDiff",
                serde_json::json!({ "threadId": thread, "toTurnCount": to }),
            )
        } else {
            (
                "orchestration.getTurnDiff",
                serde_json::json!({ "threadId": thread, "fromTurnCount": from, "toTurnCount": to }),
            )
        };
        session.call(tag, payload, move |result| {
            handle.update(move |s: &mut App, _| {
                if s.diff.seq != seq {
                    return;
                }
                s.diff.loading = false;
                match result {
                    Ok(v) => s
                        .diff
                        .state
                        .set_files(rune_kit::diff::parse(v["diff"].as_str().unwrap_or(""))),
                    Err(e) => {
                        s.diff.error = Some(e);
                        s.diff.loaded = None;
                    }
                }
            });
        });
    }

    pub fn toggle_palette(&mut self) {
        if self.palette.is_some() {
            return self.close_palette();
        }
        // The palette's field takes the keyboard; the composer gets it back on close.
        self.composer.focused = false;
        self.palette = Some(CommandPalette {
            query: focused(""),
            active: 0,
        });
        self.menu = None;
        self.popup = None;
    }

    pub fn close_palette(&mut self) {
        self.palette = None;
        self.composer.focused = true;
    }

    /// Palette rows for the current query, best matches first within each group.
    pub fn palette_items(&self) -> Vec<PaletteItem> {
        let query = self.palette.as_ref().map_or("", |p| p.query.text.trim());
        let mut actions = vec![
            PaletteItem {
                group: "Actions",
                label: "New thread".into(),
                detail: None,
                shortcut: Some("Ctrl N"),
                action: PaletteAction::NewThread,
            },
            PaletteItem {
                group: "Actions",
                label: if self.diff_pane.is_open() && self.inspector == Inspector::Changes {
                    "Hide changes"
                } else {
                    "Show changes"
                }
                .into(),
                detail: None,
                shortcut: Some("Ctrl D"),
                action: PaletteAction::ToggleDiff,
            },
            PaletteItem {
                group: "Actions",
                label: if self.diff_pane.is_open() && self.inspector == Inspector::Files {
                    "Hide files"
                } else {
                    "Show files"
                }
                .into(),
                detail: None,
                shortcut: None,
                action: PaletteAction::ToggleFiles,
            },
            PaletteItem {
                group: "Actions",
                label: if self.settings_open {
                    "Close settings"
                } else {
                    "Settings"
                }
                .into(),
                detail: None,
                shortcut: Some("Ctrl ,"),
                action: PaletteAction::Settings,
            },
            PaletteItem {
                group: "Actions",
                label: "Toggle sidebar".into(),
                detail: None,
                shortcut: Some("Ctrl Shift B"),
                action: PaletteAction::ToggleSidebar,
            },
        ];
        for (id, label) in theme::choices(&self.env_themes) {
            actions.push(PaletteItem {
                group: "Themes",
                label: format!("Theme: {label}"),
                detail: None,
                shortcut: None,
                action: PaletteAction::Theme(id),
            });
        }
        let many = self.envs.len() > 1;
        let threads = self.envs.iter().enumerate().flat_map(|(env, e)| {
            e.shell
                .threads
                .iter()
                .filter(|t| t.is_sidebar_row())
                .map(move |t| {
                    let project = e.shell.project(&t.project_id).map_or("", |p| &p.title);
                    PaletteItem {
                        group: "Threads",
                        label: t.title.clone(),
                        detail: Some(if many {
                            format!("{project} · {}", self.env_name(env))
                        } else {
                            project.to_owned()
                        }),
                        shortcut: None,
                        action: PaletteAction::OpenThread(env, t.id.clone()),
                    }
                })
        });
        let mut ranked: Vec<(usize, u8, PaletteItem)> = actions
            .into_iter()
            .chain(threads)
            .filter_map(|item| palette_rank(&item.label, query).map(|r| (r, item)))
            .enumerate()
            .map(|(i, (r, item))| (i, r, item))
            .collect();
        // Group order stays fixed; inside a group, better matches rise but keep their order.
        let group_order = |g: &str| {
            ["Actions", "Threads", "Themes"]
                .iter()
                .position(|x| *x == g)
                .unwrap_or(9)
        };
        ranked.sort_by_key(|(i, r, item)| (group_order(item.group), *r, *i));
        ranked
            .into_iter()
            .map(|(_, _, item)| item)
            .take(60)
            .collect()
    }

    pub fn run_palette(&mut self, index: usize, tx: &mut Tx) {
        let Some(item) = self.palette_items().into_iter().nth(index) else {
            return;
        };
        self.close_palette();
        match item.action {
            PaletteAction::NewThread => self.new_thread(None),
            PaletteAction::ToggleDiff => self.toggle_diff(),
            PaletteAction::ToggleFiles => self.toggle_inspector(Inspector::Files),
            PaletteAction::Settings => self.toggle_settings(),
            PaletteAction::ToggleSidebar => self.sidebar.toggle(),
            PaletteAction::OpenThread(env, id) => self.open_thread_in(env, id),
            PaletteAction::Theme(id) => self.set_theme(&id, tx),
        }
    }

    /// Arrow keys move, Return runs, Escape closes. True when taken.
    pub fn palette_key(&mut self, key: &rune::Key, tx: &mut Tx) -> bool {
        let n = self.palette_items().len();
        let Some(p) = self.palette.as_mut() else {
            return false;
        };
        match key {
            rune::Key::Escape => self.close_palette(),
            rune::Key::Down if n > 0 => p.active = (p.active + 1) % n,
            rune::Key::Up if n > 0 => p.active = (p.active + n - 1) % n,
            rune::Key::Enter => {
                let i = p.active;
                self.run_palette(i, tx);
            }
            _ => return false,
        }
        true
    }

    /// Re-reads the trigger at the composer's caret after an edit and fills the menu for it.
    pub fn refresh_menu(&mut self) {
        use t3_client::composer::{TriggerKind, detect, file_link};
        let Some(trigger) = detect(&self.composer.text, self.composer.focus) else {
            self.menu = None;
            return;
        };
        let provider = self.effective_model().and_then(|m| {
            self.config()?
                .providers
                .iter()
                .find(|p| p.instance_id == m.instance_id)
                .cloned()
        });
        let items = match trigger.kind {
            TriggerKind::Path => {
                let cwd = self
                    .thread
                    .as_ref()
                    .and_then(|t| t.thread.worktree_path.clone())
                    .or_else(|| {
                        let project = self
                            .selected
                            .as_deref()
                            .and_then(|id| self.shell().thread(id))
                            .map(|t| t.project_id.clone())
                            .or(self.draft_project.clone())?;
                        Some(self.shell().project(&project)?.workspace_root.clone())
                    });
                let (Some(cwd), Some(session), Some(handle)) =
                    (cwd, self.session(), HANDLE.get().cloned())
                else {
                    self.menu = None;
                    return;
                };
                self.menu_seq += 1;
                let seq = self.menu_seq;
                let payload =
                    serde_json::json!({ "cwd": cwd, "query": trigger.query, "limit": 20 });
                session.call("projects.searchEntries", payload, move |result| {
                    handle.update(move |s: &mut App, _| {
                        // Only the newest search may fill the menu.
                        let (true, Some(menu)) = (s.menu_seq == seq, s.menu.as_mut()) else {
                            return;
                        };
                        menu.loading = false;
                        menu.items = result
                            .ok()
                            .and_then(|v| v["entries"].as_array().cloned())
                            .unwrap_or_default()
                            .iter()
                            .filter_map(|e| {
                                let path = e["path"].as_str()?;
                                let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
                                let suffix = if e["kind"] == "directory" { "/" } else { "" };
                                Some(MenuItem {
                                    id: path.to_owned(),
                                    label: format!("{name}{suffix}"),
                                    detail: (!dir.is_empty()).then(|| dir.to_owned()),
                                    action: MenuAction::Insert(format!("{} ", file_link(path))),
                                })
                            })
                            .collect();
                        menu.active = 0;
                    });
                });
                // Keep the last results showing while the new search runs, so the list doesn't blink.
                let keep = match self.menu.take() {
                    Some(m) if m.trigger.kind == TriggerKind::Path => m.items,
                    _ => Vec::new(),
                };
                self.menu = Some(ComposerMenu {
                    trigger,
                    items: keep,
                    active: 0,
                    loading: true,
                });
                return;
            }
            TriggerKind::SlashCommand | TriggerKind::SlashModel => {
                let mut items = vec![MenuItem {
                    id: "slash:model".into(),
                    label: "/model".into(),
                    detail: Some("Switch response model for this thread".into()),
                    action: MenuAction::OpenModel,
                }];
                for c in provider.iter().flat_map(|p| &p.slash_commands) {
                    items.push(MenuItem {
                        id: format!("slash:{}", c.name),
                        label: format!("/{}", c.name),
                        detail: c.description.clone(),
                        action: MenuAction::Insert(format!("/{} ", c.name)),
                    });
                }
                let query = if trigger.kind == TriggerKind::SlashModel {
                    "model"
                } else {
                    trigger.query.as_str()
                };
                filter_items(items, query)
            }
            TriggerKind::Skill => {
                let items = provider
                    .iter()
                    .flat_map(|p| &p.skills)
                    .filter(|k| k.enabled)
                    .map(|k| MenuItem {
                        id: format!("skill:{}", k.name),
                        label: format!("${}", k.name),
                        detail: k
                            .short_description
                            .clone()
                            .or_else(|| k.description.clone()),
                        action: MenuAction::Insert(format!("${} ", k.name)),
                    })
                    .collect();
                filter_items(items, &trigger.query)
            }
            TriggerKind::PullRequest => Vec::new(),
        };
        self.menu = (!items.is_empty()).then(|| ComposerMenu {
            trigger,
            items,
            active: 0,
            loading: false,
        });
    }

    /// Applies a menu pick by item id.
    pub fn pick(&mut self, id: &str) {
        let Some(menu) = self.menu.take() else { return };
        let Some(item) = menu.items.into_iter().find(|i| i.id == id) else {
            return;
        };
        self.composer
            .set_selection(menu.trigger.start, menu.trigger.end);
        match item.action {
            MenuAction::Insert(text) => self.composer.insert(&text),
            MenuAction::OpenModel => {
                self.composer.insert("");
                self.popup = Some(Popup::Model);
            }
        }
    }

    /// Arrow keys, Return and Tab drive an open menu; Escape closes it. True when taken.
    pub fn menu_key(&mut self, key: &rune::Key) -> bool {
        let Some(menu) = self.menu.as_mut() else {
            return false;
        };
        let n = menu.items.len();
        match key {
            rune::Key::Escape => self.menu = None,
            rune::Key::Down if n > 0 => menu.active = (menu.active + 1) % n,
            rune::Key::Up if n > 0 => menu.active = (menu.active + n - 1) % n,
            rune::Key::Enter | rune::Key::Tab if n > 0 => {
                let id = menu.items[menu.active.min(n - 1)].id.clone();
                self.pick(&id);
            }
            _ => return false,
        }
        true
    }

    /// Folds the sidebar away when the window turns narrow and restores it when it widens.
    pub fn fit_width(&mut self, width: f32) {
        self.window_width = width;
        self.fit();
    }

    /// Keeps the thread at least `MAIN_MIN` wide: the inspector narrows first (to its minimum),
    /// then the sidebar folds away. A tiled half-screen window otherwise squeezed the thread,
    /// and with it the toolbar's toggles, to nothing.
    fn fit(&mut self) {
        let width = self.window_width;
        if width <= 0.0 {
            return;
        }
        let inspector = self.diff_pane.is_open();
        let sidebar_wanted = self.sidebar_before_narrow.unwrap_or(self.sidebar.is_open());
        let need = MAIN_MIN
            + if sidebar_wanted {
                self.sidebar.width()
            } else {
                0.0
            }
            + if inspector {
                self.diff_pane.min_width()
            } else {
                0.0
            };
        let narrow = width < NARROW.max(need);
        if narrow != self.narrow {
            self.narrow = narrow;
            if narrow {
                self.sidebar_before_narrow = Some(self.sidebar.is_open());
                self.sidebar.set_open(false);
            } else if let Some(open) = self.sidebar_before_narrow.take() {
                self.sidebar.set_open(open);
            }
        }
        if !inspector {
            return;
        }
        let sidebar = if self.sidebar.is_open() {
            self.sidebar.width()
        } else {
            0.0
        };
        let room = (width - sidebar - MAIN_MIN).max(self.diff_pane.min_width());
        // A width other than the one last fitted is the user's own drag: it becomes the wish.
        if self.inspector_fitted != Some(self.diff_pane.width()) {
            self.inspector_wanted = None;
        }
        let wanted = *self.inspector_wanted.get_or_insert(self.diff_pane.width());
        let fitted = wanted.min(room);
        if fitted != self.diff_pane.width() {
            self.diff_pane.set_width(fitted);
        }
        self.inspector_fitted = Some(self.diff_pane.width());
        if fitted == wanted {
            self.inspector_wanted = None;
        }
    }

    /// Connects to this machine's server through the local `t3` CLI, no link needed.
    pub fn connect_local(&mut self) {
        let Screen::Pair {
            busy,
            error,
            local: Some(_),
            ..
        } = &mut self.screen
        else {
            return;
        };
        if *busy {
            return;
        }
        *busy = true;
        *error = None;
        let (Some(handle), Some(home)) = (HANDLE.get().cloned(), pairing::local_home()) else {
            return;
        };
        std::thread::spawn(move || {
            let result = session::pair_local_blocking(&home).map_err(|e| e.to_string());
            handle.update(move |s, _| s.paired(result));
        });
    }

    fn paired(&mut self, result: Result<Credential, String>) {
        match result {
            Ok(cred) => {
                self.screen = Screen::Main;
                self.start(cred);
            }
            Err(e) => {
                if let Screen::Pair { busy, error, .. } = &mut self.screen {
                    *busy = false;
                    *error = Some(e);
                }
            }
        }
    }

    pub fn pair(&mut self) {
        let Screen::Pair {
            link, busy, error, ..
        } = &mut self.screen
        else {
            return;
        };
        if *busy || link.text.trim().is_empty() {
            return;
        }
        *busy = true;
        *error = None;
        let text = link.text.clone();
        let Some(handle) = HANDLE.get().cloned() else {
            return;
        };
        std::thread::spawn(move || {
            let result = session::pair_blocking(&text).map_err(|e| e.to_string());
            handle.update(move |s, _| s.paired(result));
        });
    }

    /// Unpairs one environment; with none left, back to pairing.
    pub fn forget(&mut self, idx: usize, tx: &mut Tx) {
        self.remove_env(idx, tx);
        if self.envs.is_empty() {
            self.screen = Screen::pairing(None);
        }
    }
}

/// Headless tooling runs (snapshots, scripts, trees) must never reach a real server.
fn headless() -> bool {
    std::env::args().any(|a| {
        matches!(
            a.as_str(),
            "--snap" | "--script" | "--tree" | "--video" | "--frames" | "--trace"
        )
    })
}

pub fn focused(text: &str) -> TextEdit {
    let mut e = TextEdit::new(text);
    e.focused = true;
    e
}

/// Toasts expire on the wake their creation scheduled.
/// Runs before every frame and on `wake_after`; returns true only when something visible changed.
fn poll(s: &mut App, _now: f64, tx: &mut Tx) -> bool {
    DIAG_POLLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now = now_secs();
    let before = s.toasts.len();
    s.toasts.retain(|t| t.2 > now);
    let mut changed = s.toasts.len() != before;
    let working = s
        .envs
        .iter()
        .any(|e| e.shell.threads.iter().any(|t| t.is_working()))
        || s.thread.as_ref().is_some_and(|t| t.is_working());
    if working && now != s.clock {
        s.clock = now;
        changed = true;
    }
    s.sync_files();
    s.check_queued_edit(tx);
    // A draft headed for a worktree needs its project's branches for the base picker.
    if s.selected.is_none() && s.draft_in_worktree() {
        s.load_refs();
    }
    if let Some(wait) = s.visit_due() {
        tx.wake_after(wait);
    }
    // A snoozed thread returns to its folder when it wakes, with no server event to say so.
    let now_ms = now * 1000;
    if s.snooze_wake.is_some_and(|w| w <= now_ms) {
        changed = true;
        // The open thread may have just woken: its visit moves to the wake.
        s.track_visit();
    }
    s.snooze_wake = s
        .envs
        .iter()
        .flat_map(|e| &e.shell.threads)
        .filter_map(|t| {
            t.snoozed_until
                .as_deref()
                .and_then(t3_client::shelves::iso_ms)
        })
        .filter(|w| *w > now_ms)
        .min();
    if let Some(w) = s.snooze_wake {
        tx.wake_after(std::time::Duration::from_millis((w - now_ms) as u64 + 50));
    }
    if working || !s.toasts.is_empty() {
        tx.wake_after(std::time::Duration::from_secs(1));
    }
    if changed {
        DIAG_CHANGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    changed
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

static DIAG_EVENTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static DIAG_POLLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static DIAG_CHANGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `T3_RUNE_FRAMES=1` prints frames drawn per second to stderr: an idle window should print 0.
fn frame_counter() -> impl FnMut(&rune::FrameEncoded) {
    let (mut window, mut drawn) = (std::time::Instant::now(), 0u32);
    move |f| {
        if matches!(f.outcome(), rune::FrameOutcome::Drawn) {
            drawn += 1;
        }
        if window.elapsed() >= std::time::Duration::from_secs(1) {
            let events = DIAG_EVENTS.swap(0, std::sync::atomic::Ordering::Relaxed);
            let polls = DIAG_POLLS.swap(0, std::sync::atomic::Ordering::Relaxed);
            let changed = DIAG_CHANGED.swap(0, std::sync::atomic::Ordering::Relaxed);
            eprintln!(
                "t3-rune: {drawn} frames drawn in the last second ({events} server events, {polls} polls, {changed} changed)"
            );
            (window, drawn) = (std::time::Instant::now(), 0);
        }
    }
}

fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    use rune::Key;
    // Escape closes the innermost thing first; the inspector is last, so a squeezed window
    // always has a way out.
    if matches!(ev.key, Key::Escape) {
        if s.find.is_some() {
            s.close_find();
        } else if s.editing_queued.is_some() {
            s.cancel_queued_edit();
        } else if s.rename.is_some() {
            s.finish_rename(false);
        } else if s.popup.is_some() {
            s.popup = None;
        } else if s.settings_open {
            s.toggle_settings();
        } else if s.diff_pane.is_open() && s.inspector == Inspector::Files && s.files.open.is_some()
        {
            s.close_file();
        } else if s.diff_pane.is_open() {
            s.diff_pane.set_open(false);
            s.fit();
        } else {
            return false;
        }
        return true;
    }
    // Rune's own: settings and stop, which the web has no binding for.
    if ev.is_command(',') {
        s.toggle_settings();
        return true;
    }
    if ev.is_command('.') {
        s.stop();
        return true;
    }
    // Everything else follows the server's keybindings, the user's own included.
    if s.env().is_some_and(|e| !e.keys.is_empty()) {
        return s.run_shortcut(ev, tx);
    }
    // No server has sent its keybindings yet: the web's defaults for the few that matter.
    let ctrl = ev.mods.cmd || ev.mods.ctrl;
    if ev.is_command('k') {
        s.toggle_palette();
    } else if ctrl && !ev.mods.shift && tx.physical_key() == Some(rune::PhysicalKeyCode::KeyD) {
        s.toggle_diff();
    } else if shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
    } else if ev.is_command('n') && !ev.mods.shift {
        s.new_thread(None);
    } else {
        return false;
    }
    true
}

/// Headless scenes over captured wire data (`T3_RUNE_FIXTURE`, written by
/// `cargo run -p t3-client --example capture`): the same decoders and reducers a live session
/// uses, with no network.
fn load_fixture(s: &mut App) {
    let Some(raw) = std::env::var_os("T3_RUNE_FIXTURE").and_then(|p| std::fs::read(p).ok()) else {
        return;
    };
    let Ok(fixture) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return;
    };
    let items = |k: &str| fixture[k].as_array().cloned().unwrap_or_default();
    // One captured environment, as if paired locally.
    s.envs = vec![Env {
        cred: Credential {
            http_base: "http://127.0.0.1:1/".into(),
            environment_id: "fixture".into(),
            label: "fixture".into(),
            access_token: String::new(),
        },
        session: None,
        status: Status::Connected,
        config: None,
        shell: ShellState::default(),
        themes: Vec::new(),
        generation: 0,
        watch: Default::default(),
        keys: Vec::new(),
    }];
    s.focus = 0;
    let mut themes = Vec::new();
    let env = &mut s.envs[0];
    for v in items("config") {
        match serde_json::from_value::<ServerConfigEvent>(v) {
            Ok(ServerConfigEvent::Snapshot { config }) => env.config = Some(config),
            Ok(ServerConfigEvent::EnvironmentThemesUpdated { payload }) => {
                themes = payload
                    .themes
                    .iter()
                    .filter_map(t3_theme::environment_theme)
                    .collect();
            }
            _ => {}
        }
    }
    for v in items("shell") {
        if let Ok(item) = serde_json::from_value(v) {
            env.shell.apply(item);
        }
    }
    for v in items("thread") {
        if let ThreadStreamItem::Snapshot {
            sequence,
            projection,
        } = ThreadStreamItem::decode(v)
        {
            s.selected = Some(projection.thread.id.clone());
            s.thread = Some(ThreadState::from_snapshot(sequence, *projection));
        }
    }
    s.env_themes = themes;
    s.screen = Screen::Main;
    s.rebuild_looks();
}

/// A scene's theme, in memory only: scenes never write the user's prefs.
fn scene_theme(s: &mut App, theme: &str, mode: &str) {
    load_fixture(s);
    s.prefs.theme = theme.to_owned();
    s.prefs.mode = mode.to_owned();
    s.rebuild_looks();
}

/// The fixture thread with the diff panel open on the turn the capture fetched.
fn scene_diff(s: &mut App) {
    scene_theme(s, "omarchy", "dark");
    let Some(raw) = std::env::var_os("T3_RUNE_FIXTURE").and_then(|p| std::fs::read(p).ok()) else {
        return;
    };
    let Ok(fixture) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return;
    };
    let diff = &fixture["diff"];
    let (Some(to), Some(patch)) = (diff["to"].as_u64(), diff["patch"].as_str()) else {
        return;
    };
    s.diff.sel = DiffSel::Turn(to);
    s.diff.state.set_files(rune_kit::diff::parse(patch));
    s.diff.loaded = s
        .thread
        .as_ref()
        .map(|t| (t.thread.id.clone(), diff["from"].as_u64().unwrap_or(0), to));
    s.diff_pane.set_open(true);
}

/// The Files tab over this crate's own folder, read from the disk.
fn scene_files(s: &mut App, file: Option<&str>) {
    scene_theme(s, "omarchy", "dark");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    s.inspector = Inspector::Files;
    s.diff_pane.set_open(true);
    s.files.scene(root, &["crates", "crates/t3-rune"], file);
}

/// The fixture thread with its last command expanded, showing styled output.
fn scene_command_output(s: &mut App) {
    scene_theme(s, "omarchy", "dark");
    let Some(id) = s
        .thread
        .as_ref()
        .and_then(|t| {
            t.visible
                .iter()
                .rev()
                .find(|r| r.item.kind == "command_execution")
        })
        .map(|r| r.item.id.clone())
    else {
        return;
    };
    let mut out = t3_client::terminal::TerminalOutput::default();
    out.append("\u{1b}[1m\u{1b}[32m   Compiling\u{1b}[0m t3-client v0.1.0\n\u{1b}[1m\u{1b}[33mwarning\u{1b}[0m\u{1b}[1m: unused variable: `x`\u{1b}[0m\n  \u{1b}[34m-->\u{1b}[0m src/rpc.rs:42:9\n\u{1b}[1m\u{1b}[32m    Finished\u{1b}[0m `dev` profile in 3.21s\nDownloading 10%\rDownloading 100%\n\u{1b}[7m PASS \u{1b}[0m 24 tests\n");
    s.outputs.insert(
        id,
        CommandOutput {
            out,
            running: false,
            loaded: true,
            error: None,
        },
    );
}

pub const SCENES: &[(&str, fn(&mut App))] = &[
    ("command-output", scene_command_output),
    ("diff", scene_diff),
    ("palette", |s| {
        scene_theme(s, "omarchy", "dark");
        s.toggle_palette();
    }),
    ("files", |s| scene_files(s, None)),
    ("file", |s| {
        scene_files(s, Some("crates/t3-rune/src/files.rs"))
    }),
    ("settings", |s| {
        scene_theme(s, "omarchy", "dark");
        s.toggle_settings();
    }),
    ("fixture", load_fixture),
    ("omarchy", |s| scene_theme(s, "omarchy", "dark")),
    ("t3-light", |s| scene_theme(s, "light", "light")),
    ("t3-dark", |s| scene_theme(s, "dark", "dark")),
    ("grove-light", |s| scene_theme(s, "grove", "light")),
    ("ember-dark", |s| scene_theme(s, "ember", "dark")),
];

fn main() {
    // Window managers match the window by this (Hyprland's `class`, a .desktop file's StartupWMClass).
    rune::set_app_id("com.t3tools.T3Code.Rune");
    rune::prewarm_gpu();
    let mut fonts: Vec<&'static [u8]> = rune_theme_opencode::FONTS.to_vec();
    fonts.shrink_to_fit();
    let spec = AppSpec {
        on_key: Some(on_key),
        poll: Some(poll),
        fonts: Box::leak(fonts.into_boxed_slice()),
        families: rune_theme_opencode::FAMILIES,
        scenes: SCENES,
        min_size: Some((720.0, 480.0)),
        ..AppSpec::new("T3 Code", (1280.0, 820.0), App::new, view::root)
    };
    let chrome = Chrome {
        full_size_content: true,
        traffic_lights: Some((16.0, 25.0)),
        drag_regions: true,
    };
    let mut app = rune::App::new(spec).chrome(chrome);
    if std::env::var_os("T3_RUNE_FRAMES").is_some() {
        app = app.on_frame_encoded(frame_counter());
        // Names what keeps a resting window drawing: looping or moving elements, once a second.
        let mut last = std::time::Instant::now();
        app = app.on_frame_drawn(move |rt: &rune::Runtime<App>| {
            if last.elapsed() < std::time::Duration::from_secs(1) {
                return;
            }
            last = std::time::Instant::now();
            let busy: Vec<String> = rt
                .elements()
                .into_iter()
                .filter(|e| e.moving || e.loop_phase.is_some() || e.revealing || e.in_growing_row)
                .map(|e| {
                    format!(
                        "{}{}",
                        e.key.unwrap_or(e.path),
                        if e.loop_phase.is_some() {
                            " (loop)"
                        } else {
                            ""
                        }
                    )
                })
                .take(8)
                .collect();
            if !busy.is_empty() {
                eprintln!("t3-rune: animating: {}", busy.join(", "));
            }
        });
    }
    let _ = HANDLE.set(app.handle());
    app.run();
}
