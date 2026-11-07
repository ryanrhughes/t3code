//! T3 Code on Rune: a native client for any T3 server (local, LAN, tailnet or tunnel).
//! `cargo run -p t3-rune` opens the window; `-- --snap out.png --scene <name>` renders headless.
mod prefs;
mod theme;
mod view;

use std::collections::{HashMap, HashSet};
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
#[derive(Clone, PartialEq)]
pub enum Popup {
    Model,
    Theme,
    Project,
}

pub struct App {
    pub screen: Screen,
    pub cred: Option<Credential>,
    pub session: Option<SessionHandle>,
    pub status: Status,
    pub config: Option<ServerConfig>,
    pub env_themes: Vec<ThemeDefinition>,
    pub prefs: prefs::Prefs,
    pub looks: theme::Looks,
    pub shell: ShellState,
    pub sidebar: PaneState,
    pub collapsed: HashSet<String>,
    /// The open thread, or a draft in `draft_project` when None.
    pub selected: Option<String>,
    pub draft_project: Option<String>,
    pub thread: Option<ThreadState>,
    pub composer: TextEdit,
    /// The model the next send uses; None follows the thread's.
    pub model: Option<ModelSelection>,
    pub popup: Option<Popup>,
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
}

impl App {
    fn new() -> App {
        let prefs = prefs::Prefs::load();
        let looks = theme::Looks::build(&prefs.theme(), &[]);
        let cred = pairing::load_credential();
        let mut app = App {
            screen: if cred.is_some() {
                Screen::Main
            } else {
                Screen::pairing(None)
            },
            cred: None,
            session: None,
            status: Status::Connecting,
            config: None,
            env_themes: Vec::new(),
            prefs,
            looks,
            shell: ShellState::default(),
            sidebar: PaneState::new(276.0).range(200.0, 520.0),
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
        };
        match cred {
            Some(c) => app.start(c),
            // First launch on a machine that runs T3 Code: connect to it with no link. A revoked or
            // forgotten pairing lands on the pairing screen instead, so revoking sticks.
            None if matches!(app.screen, Screen::Pair { local: Some(_), .. }) && !headless() => {
                app.connect_local()
            }
            None => {}
        }
        app
    }

    /// Connects in the background. Events land through `AppHandle::update`, so nothing polls.
    fn start(&mut self, cred: Credential) {
        self.cred = Some(cred.clone());
        let Some(handle) = HANDLE.get().cloned() else {
            return;
        };
        self.session = Some(session::spawn(
            cred,
            std::sync::Arc::new(move |ev| handle.update(move |s, tx| s.on_event(ev, tx))),
        ));
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

    fn on_event(&mut self, ev: ClientEvent, tx: &mut Tx) {
        DIAG_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        match ev {
            ClientEvent::Status(s) => self.status = s,
            ClientEvent::Config(ev) => match ev {
                ServerConfigEvent::Snapshot { config } => {
                    // `t3 theme set <id>` is adopted once per generation, as the web does.
                    if let Some(generation) = config.default_theme() {
                        if self.prefs.adopt_default_theme(&generation) {
                            self.rebuild_looks();
                        }
                    }
                    self.config = Some(config);
                }
                ServerConfigEvent::ProviderStatuses { payload } => {
                    if let Some(c) = self.config.as_mut() {
                        c.providers = payload.providers;
                    }
                }
                ServerConfigEvent::SettingsUpdated { payload } => {
                    if let Some(c) = self.config.as_mut() {
                        c.settings = payload.settings;
                        if let Some(generation) = c.default_theme() {
                            if self.prefs.adopt_default_theme(&generation) {
                                self.rebuild_looks();
                            }
                        }
                    }
                }
                ServerConfigEvent::EnvironmentThemesUpdated { payload } => {
                    // Omarchy rewrites themes/omarchy.json on every theme switch; this repaints live.
                    self.env_themes = payload
                        .themes
                        .iter()
                        .filter_map(t3_theme::environment_theme)
                        .collect();
                    tx.color_motion(rune::Spring::new(0.4, 0.0));
                    self.rebuild_looks();
                }
                ServerConfigEvent::Unknown => {}
            },
            ClientEvent::Shell(item) => {
                // The first snapshot lands without motion so the sidebar doesn't animate in.
                if !self.shell.loaded {
                    tx.instant();
                }
                self.shell.apply(item);
                if self.selected.is_none() && self.draft_project.is_none() {
                    self.draft_project =
                        self.shell.sidebar_projects().first().map(|p| p.id.clone());
                }
            }
            ClientEvent::Thread { thread_id, item } => {
                if self.selected.as_deref() != Some(&thread_id) {
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
                    }
                    ThreadStreamItem::Event {
                        sequence,
                        thread_id,
                        event,
                    } => {
                        if let Some(t) = self.thread.as_mut() {
                            t.apply(sequence, &thread_id, event);
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
            ClientEvent::ThreadLaunched { thread_id } => self.open_thread(Some(thread_id)),
            ClientEvent::CommandFailed(msg) => self.toast(msg, tx),
            ClientEvent::Unauthorized(msg) => {
                pairing::forget_credential();
                self.session = None;
                self.screen = Screen::pairing(Some(format!("Pair again: {msg}")));
            }
        }
    }

    pub fn open_thread(&mut self, id: Option<String>) {
        if self.selected == id {
            return;
        }
        self.thread = None;
        self.model = None;
        self.popup = None;
        if let Some(t) = id.as_deref().and_then(|id| self.shell.thread(id)) {
            self.draft_project = Some(t.project_id.clone());
        }
        self.selected = id.clone();
        if let Some(s) = &self.session {
            s.watch_thread(id);
        }
        self.composer.focused = true;
    }

    pub fn new_thread(&mut self, project_id: Option<String>) {
        let project = project_id
            .or_else(|| self.draft_project.clone())
            .or_else(|| self.shell.sidebar_projects().first().map(|p| p.id.clone()));
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
            .and_then(|id| self.shell.thread(id))
        {
            return Some(t.model_selection.clone());
        }
        let config = self.config.as_ref()?;
        let project_default = self
            .draft_project
            .as_deref()
            .and_then(|p| self.shell.project(p))
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

    pub fn send(&mut self, tx: &mut Tx) {
        let text = self.composer.text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let Some(session) = &self.session else { return };
        let Some(model) = self.effective_model() else {
            self.toast("No provider is ready on this server", tx);
            return;
        };
        match self.selected.clone() {
            Some(id) => {
                let active = self
                    .thread
                    .as_ref()
                    .is_some_and(|t| t.active_run().is_some());
                session.dispatch(protocol::message_dispatch(
                    &id,
                    &text,
                    self.model.as_ref(),
                    active,
                ));
            }
            None => {
                let Some(project) = self.draft_project.clone() else {
                    self.toast("Pick a project first", tx);
                    return;
                };
                let runtime = self
                    .config
                    .as_ref()
                    .map_or("full-access".to_owned(), |c| c.default_runtime_mode());
                session.launch(protocol::launch_thread(&project, &text, &model, &runtime));
            }
        }
        self.composer = focused("");
    }

    pub fn stop(&mut self) {
        let (Some(session), Some(t)) = (&self.session, &self.thread) else {
            return;
        };
        let thread_stop = self
            .config
            .as_ref()
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
        let (Some(session), Some(id)) = (&self.session, &self.selected) else {
            return;
        };
        session.dispatch(protocol::respond_decision(id, request_id, decision));
    }

    pub fn answer(&mut self, request_id: &str) {
        let (Some(session), Some(id)) = (&self.session, &self.selected) else {
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

    pub fn forget(&mut self) {
        pairing::forget_credential();
        self.session = None;
        self.cred = None;
        self.config = None;
        self.shell = ShellState::default();
        self.thread = None;
        self.selected = None;
        self.screen = Screen::pairing(None);
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
    let working = s.shell.threads.iter().any(|t| t.is_working())
        || s.thread.as_ref().is_some_and(|t| t.is_working());
    if working && now != s.clock {
        s.clock = now;
        changed = true;
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
    if shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.is_command('n') && !ev.mods.shift {
        s.new_thread(None);
        return true;
    }
    if ev.is_command('.') {
        s.stop();
        return true;
    }
    if matches!(ev.key, rune::Key::Escape) && s.popup.is_some() {
        s.popup = None;
        return true;
    }
    let _ = tx;
    false
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
    for v in items("config") {
        match serde_json::from_value::<ServerConfigEvent>(v) {
            Ok(ServerConfigEvent::Snapshot { config }) => s.config = Some(config),
            Ok(ServerConfigEvent::EnvironmentThemesUpdated { payload }) => {
                s.env_themes = payload
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
            s.shell.apply(item);
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
    s.screen = Screen::Main;
    s.status = Status::Connected;
    s.rebuild_looks();
}

/// A scene's theme, in memory only: scenes never write the user's prefs.
fn scene_theme(s: &mut App, theme: &str, mode: &str) {
    load_fixture(s);
    s.prefs.theme = theme.to_owned();
    s.prefs.mode = mode.to_owned();
    s.rebuild_looks();
}

pub const SCENES: &[(&str, fn(&mut App))] = &[
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
