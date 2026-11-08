//! Main-thread state and event handlers. Every entry point (hotkey, FSEvents, AppKit
//! delegates) runs on the main thread and goes through `with_state`, which skips
//! re-entrant calls: AppKit can fire `windowDidResignKey` while we are hiding the window.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Command as Proc, Stdio};

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, sel};
use block2::RcBlock;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEventModifierFlags, NSPasteboard, NSPasteboardTypeString, NSRunningApplication, NSWorkspace,
    NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{NSError, NSString, NSURL};

use crate::calc;
use crate::catalog::{Action, Catalog, History, Icon, Item};
use crate::config::{self, Config, ConfigError};
use crate::hotkey;
use crate::ui::{Controller, Ui};
use crate::watch::Watcher;

pub struct State {
    mtm: MainThreadMarker,
    dir: PathBuf,
    cfg: Config,
    cfg_err: Option<ConfigError>,
    catalog: Catalog,
    history: History,
    controller: Retained<Controller>,
    ui: Ui,
    hotkey: Option<hotkey::Registration>,
    results: Vec<Item>,
    selected: usize,
    /// id of a `confirm = true` command waiting for a second Enter.
    armed: Option<String>,
    config_dirs: Vec<PathBuf>,
    _config_watch: Option<Watcher>,
    apps_watch: Option<Watcher>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

pub fn log(msg: impl AsRef<str>) {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    eprintln!("[{t}] {}", msg.as_ref());
}

/// Ranked results for a query: config error first, then calculator, then catalog matches.
pub fn compute_results(
    q: &str,
    cfg: &Config,
    err: Option<&ConfigError>,
    catalog: &mut Catalog,
    history: &History,
    limit: usize,
) -> Vec<Item> {
    let mut out = Vec::new();
    if let Some(e) = err {
        let file = e.file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(Item::transient(
            format!("Config error in {file}. Press Enter to open it."),
            e.message.lines().next().unwrap_or("").to_string(),
            Icon::Symbol("exclamationmark.triangle.fill".into()),
            Action::EditFile(e.file.clone()),
        ));
    }
    if cfg.main.calculator.enabled {
        if let Some(v) = calc::try_query(q) {
            let s = calc::format(v, cfg.main.calculator.precision);
            out.push(Item::transient(
                s.clone(),
                format!("{} = {s}    Enter copies the result", q.trim().trim_start_matches('=')),
                Icon::Symbol("equal.circle".into()),
                Action::Copy(s),
            ));
        }
    }
    let rest = limit.saturating_sub(out.len());
    out.extend(catalog.search(q, cfg, history, rest));
    out
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Launch an app as an application, not as a document. `openURL` on a bundle's file URL
/// goes through LaunchServices' document path, which can write to the bundle on our
/// behalf and makes macOS App Management show "raylauncher was prevented from modifying
/// apps on your Mac". The application path (what `open -a` uses) does not.
fn open_app(p: &Path) {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
    let config = NSWorkspaceOpenConfiguration::configuration();
    let path = p.to_path_buf();
    // Called on a background queue; `log` only writes to stderr.
    let done = RcBlock::new(move |_app: *mut NSRunningApplication, err: *mut NSError| {
        // SAFETY: AppKit passes either null or a valid NSError for the duration of the call.
        if let Some(err) = unsafe { err.as_ref() } {
            log(format!("could not open {}: {}", path.display(), err.localizedDescription()));
        }
    });
    NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(&url, &config, Some(&done));
}

fn canonical(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

/// Folders to watch for config edits: the config folder plus, when config files are
/// symlinks into a dotfiles repo, the folders that hold the real files.
fn config_watch_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = canonical(dir).into_iter().collect();
    for f in [config::MAIN_FILE, config::COMMANDS_FILE, config::SYSTEM_FILE] {
        if let Some(parent) = canonical(&dir.join(f)).and_then(|p| p.parent().map(Path::to_path_buf)) {
            if !dirs.contains(&parent) {
                dirs.push(parent);
            }
        }
    }
    dirs
}

fn watch_apps(cfg: &Config) -> Option<Watcher> {
    let dirs: Vec<PathBuf> = cfg.main.search.app_dirs.iter().filter_map(|d| canonical(&config::expand_tilde(d))).collect();
    Watcher::new(&dirs, 2.0, |_| {
        with_state(|s| s.catalog.apps_dirty = true);
    })
}

impl State {
    fn refresh(&mut self) {
        let q = self.ui.query();
        let limit = self.ui.max_rows();
        self.results = compute_results(&q, &self.cfg, self.cfg_err.as_ref(), &mut self.catalog, &self.history, limit);
        self.selected = 0;
        self.armed = None;
        self.render();
    }

    fn render(&mut self) {
        let note = self.armed.as_ref().map(|_| "Press Enter again to confirm, Esc to cancel");
        self.ui.render(&self.results, self.selected, note);
    }

    fn toggle(&mut self) {
        if self.ui.is_visible() {
            self.ui.hide(self.mtm);
        } else {
            self.catalog.refresh_apps(&self.cfg);
            self.ui.show(self.mtm);
            self.refresh();
        }
    }

    fn hide(&mut self) {
        self.armed = None;
        self.ui.hide(self.mtm);
    }

    fn move_selection(&mut self, delta: isize) {
        let n = self.results.len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(n as isize) as usize;
        self.armed = None;
        self.render();
    }

    fn run_shell(&self, cmd: &str) {
        let sh = &self.cfg.main.shell;
        let res = Proc::new(&sh[0])
            .args(&sh[1..])
            .arg(cmd)
            .current_dir(config::home())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn();
        if let Err(e) = res {
            log(format!("failed to run {cmd:?} with {sh:?}: {e}"));
        }
    }

    fn edit(&self, file: &Path) {
        let cmd = format!("{} {}", self.cfg.main.editor, shell_quote(&file.to_string_lossy()));
        self.run_shell(&cmd);
    }

    fn activate(&mut self, index: usize, alternate: bool) {
        let Some(item) = self.results.get(index).cloned() else { return };
        match &item.action {
            Action::OpenApp(p) if alternate => self.run_shell(&format!("open -R {}", shell_quote(&p.to_string_lossy()))),
            Action::OpenApp(p) => open_app(p),
            Action::Run { cmd, confirm } => {
                if *confirm && self.armed.as_deref() != Some(item.id.as_str()) {
                    self.selected = index;
                    self.armed = Some(item.id.clone());
                    self.render();
                    return;
                }
                self.run_shell(cmd);
            }
            Action::Copy(s) => {
                let pb = NSPasteboard::generalPasteboard();
                pb.clearContents();
                pb.setString_forType(&NSString::from_str(s), unsafe { NSPasteboardTypeString });
            }
            Action::EditFile(f) => self.edit(f),
            Action::Builtin(b) => match b.as_str() {
                "@quit" => {
                    log("quit requested");
                    std::process::exit(0);
                }
                "@edit-config" => self.edit(&self.dir.join(config::MAIN_FILE)),
                "@reindex" => {
                    self.catalog.apps_dirty = true;
                    self.catalog.refresh_apps(&self.cfg);
                    log(format!("reindexed {} apps", self.catalog.apps.len()));
                }
                other => log(format!("unknown built-in {other}")),
            },
        }
        if self.cfg.main.search.history {
            self.history.bump(&item.id);
        }
        self.hide();
    }

    fn reload_config(&mut self) {
        let new = match config::load(&self.dir) {
            Ok(c) => c,
            Err(e) => {
                log(format!("config error, keeping previous settings: {e}"));
                self.cfg_err = Some(e);
                if self.ui.is_visible() {
                    self.refresh();
                }
                return;
            }
        };
        self.cfg_err = None;
        if new.hotkey != self.cfg.hotkey {
            match hotkey::register(new.hotkey) {
                Ok(r) => self.hotkey = Some(r), // drops and unregisters the old one
                Err(e) => {
                    self.cfg_err = Some(ConfigError { file: self.dir.join(config::MAIN_FILE), message: e });
                }
            }
        }
        if new.main.window != self.cfg.main.window {
            let visible = self.ui.is_visible();
            self.ui.hide(self.mtm);
            self.ui.close();
            self.ui = Ui::build(self.mtm, &new.main.window, &self.controller);
            if visible {
                self.ui.show(self.mtm);
            }
        }
        if new.main.search.app_dirs != self.cfg.main.search.app_dirs {
            self.apps_watch = watch_apps(&new);
        }
        self.catalog.apply_config(&new);
        self.cfg = new;
        self.catalog.refresh_apps(&self.cfg);
        log(format!(
            "config reloaded: {} apps, {} commands, hotkey {}",
            self.catalog.apps.len(),
            self.catalog.commands.len(),
            self.cfg.main.hotkey
        ));
        if self.ui.is_visible() {
            self.refresh();
        }
    }
}

// ---- entry points called from AppKit / Carbon / FSEvents ----

fn on_hotkey() {
    with_state(State::toggle);
}

pub fn on_remote_toggle() {
    with_state(State::toggle);
}

pub fn on_query_changed() {
    with_state(State::refresh);
}

pub fn on_resign_key() {
    with_state(State::hide);
}

pub fn on_row_click(i: usize) {
    with_state(|s| s.activate(i, false));
}

pub fn on_command(sel: Sel) -> bool {
    with_state(|s| {
        if sel == sel!(moveUp:) || sel == sel!(insertBacktab:) {
            s.move_selection(-1);
        } else if sel == sel!(moveDown:) || sel == sel!(insertTab:) {
            s.move_selection(1);
        } else if sel == sel!(insertNewline:) {
            let cmd = NSApplication::sharedApplication(s.mtm)
                .currentEvent()
                .is_some_and(|e| e.modifierFlags().contains(NSEventModifierFlags::Command));
            s.activate(s.selected, cmd);
        } else if sel == sel!(cancelOperation:) {
            s.hide();
        } else {
            return false;
        }
        true
    })
    .unwrap_or(false)
}

fn on_config_files_changed(paths: Vec<PathBuf>) {
    with_state(|s| {
        let relevant = paths.iter().any(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            s.config_dirs.iter().any(|d| p.starts_with(d))
                && [config::MAIN_FILE, config::COMMANDS_FILE, config::SYSTEM_FILE].contains(&name)
        });
        if relevant {
            s.reload_config();
        }
    });
}

unsafe extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
    fn flock(fd: i32, op: i32) -> i32;
}

/// Hold an exclusive lock for the life of the process so a second copy exits.
fn single_instance(state: &Path) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    std::fs::create_dir_all(state).ok()?;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(state.join("raylauncher.lock")).ok()?;
    (unsafe { flock(f.as_raw_fd(), 2 | 4) } == 0).then_some(f) // LOCK_EX | LOCK_NB
}

pub fn run() {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");
    let state_dir = config::state_dir();
    let Some(_lock) = single_instance(&state_dir) else {
        eprintln!("raylauncher is already running (lock held on {})", state_dir.join("raylauncher.lock").display());
        std::process::exit(1);
    };
    // Spawned commands are never waited on; let the kernel reap them.
    unsafe { signal(20, 1) }; // SIGCHLD, SIG_IGN

    let dir = config::config_dir();
    if let Err(e) = config::ensure_files(&dir) {
        log(format!("could not write default config to {}: {e}", dir.display()));
    }
    let (cfg, cfg_err) = match config::load(&dir) {
        Ok(c) => (c, None),
        Err(e) => {
            log(format!("config error, using built-in defaults: {e}"));
            (config::builtin(), Some(e))
        }
    };

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let controller = Controller::new(mtm);
    let ui = Ui::build(mtm, &cfg.main.window, &controller);
    let mut catalog = Catalog::new(&cfg);
    catalog.refresh_apps(&cfg);
    // Loaded even when history is off, so turning it on later starts from saved counts.
    let history = History::load(state_dir.join("history.tsv"));

    hotkey::install(on_hotkey);
    let (hk, cfg_err) = match hotkey::register(cfg.hotkey) {
        Ok(r) => (Some(r), cfg_err),
        Err(e) => {
            log(&e);
            (None, cfg_err.or(Some(ConfigError { file: dir.join(config::MAIN_FILE), message: e })))
        }
    };

    let config_dirs = config_watch_dirs(&dir);
    let config_watch = Watcher::new(&config_dirs, 0.15, on_config_files_changed);
    let apps_watch = watch_apps(&cfg);

    log(format!(
        "started: config {}, {} apps, {} commands, hotkey {}",
        dir.display(),
        catalog.apps.len(),
        catalog.commands.len(),
        cfg.main.hotkey
    ));

    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            mtm,
            dir,
            cfg,
            cfg_err,
            catalog,
            history,
            controller,
            ui,
            hotkey: hk,
            results: Vec::new(),
            selected: 0,
            armed: None,
            config_dirs,
            _config_watch: config_watch,
            apps_watch,
        })
    });
    app.run();
}
