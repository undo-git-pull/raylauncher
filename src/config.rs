//! Config loading. The files in `defaults/` are the single source of truth: they are
//! compiled into the binary, written to the config folder on first run, and used as
//! the base that a (possibly partial) user `raylauncher.toml` is merged onto.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

pub const DEFAULT_MAIN: &str = include_str!("../defaults/raylauncher.toml");
pub const DEFAULT_COMMANDS: &str = include_str!("../defaults/commands.toml");
pub const DEFAULT_SYSTEM: &str = include_str!("../defaults/system.toml");
pub const AGENTS_MD: &str = include_str!("../defaults/AGENTS.md");

pub const MAIN_FILE: &str = "raylauncher.toml";
pub const COMMANDS_FILE: &str = "commands.toml";
pub const SYSTEM_FILE: &str = "system.toml";
pub const AGENTS_FILE: &str = "AGENTS.md";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Main {
    pub hotkey: String,
    pub shell: Vec<String>,
    pub editor: String,
    pub window: Window,
    pub search: Search,
    pub calculator: Calculator,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub width: f64,
    pub max_results: usize,
    pub font_size: f64,
    pub top_offset: f64,
    pub theme: String,
    pub show_icons: bool,
    pub placeholder: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub app_dirs: Vec<String>,
    pub scan_depth: usize,
    pub exclude: Vec<String>,
    pub prefer_prefix: bool,
    pub history: bool,
    pub history_weight: f64,
    pub show_recent: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calculator {
    pub enabled: bool,
    pub precision: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub name: String,
    pub run: String,
    #[serde(default)]
    pub keywords: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandFile {
    #[serde(default)]
    command: Vec<Command>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub main: Main,
    pub hotkey: crate::hotkey::Combo,
    /// Commands from commands.toml followed by system.toml, enabled ones only.
    pub commands: Vec<Command>,
}

/// A config problem, tied to the file that caused it so the UI can offer to open it.
#[derive(Debug, Clone)]
pub struct ConfigError {
    pub file: PathBuf,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file.display(), self.message.trim_end())
    }
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()))
}

pub fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~") {
        Some(rest) => home().join(rest.trim_start_matches('/')),
        None => PathBuf::from(p),
    }
}

/// `$RAYLAUNCHER_CONFIG_DIR`, else `$XDG_CONFIG_HOME/raylauncher`, else `~/.config/raylauncher`.
pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("RAYLAUNCHER_CONFIG_DIR") {
        return d.into();
    }
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x).join("raylauncher"),
        _ => home().join(".config/raylauncher"),
    }
}

/// `$RAYLAUNCHER_STATE_DIR`, else `$XDG_STATE_HOME/raylauncher`, else `~/.local/state/raylauncher`.
pub fn state_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("RAYLAUNCHER_STATE_DIR") {
        return d.into();
    }
    match std::env::var_os("XDG_STATE_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x).join("raylauncher"),
        _ => home().join(".local/state/raylauncher"),
    }
}

/// Create missing config files from the built-in defaults. Existing files are never
/// touched, except AGENTS.md, which documents the running version and is kept current.
pub fn ensure_files(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, body) in [(MAIN_FILE, DEFAULT_MAIN), (COMMANDS_FILE, DEFAULT_COMMANDS), (SYSTEM_FILE, DEFAULT_SYSTEM)] {
        let p = dir.join(name);
        if !p.exists() {
            std::fs::write(&p, body)?;
        }
    }
    let agents = dir.join(AGENTS_FILE);
    if std::fs::read_to_string(&agents).ok().as_deref() != Some(AGENTS_MD) {
        std::fs::write(&agents, AGENTS_MD)?;
    }
    Ok(())
}

pub fn load(dir: &Path) -> Result<Config, ConfigError> {
    load_from(dir, |name| read_optional(&dir.join(name)))
}

/// The built-in defaults, used when the user's files are broken at startup.
pub fn builtin() -> Config {
    load_from(Path::new("<built-in>"), |name| {
        Ok(match name {
            MAIN_FILE => DEFAULT_MAIN,
            COMMANDS_FILE => DEFAULT_COMMANDS,
            _ => DEFAULT_SYSTEM,
        }
        .to_string())
    })
    .expect("built-in config is valid")
}

fn load_from(dir: &Path, read: impl Fn(&str) -> Result<String, String>) -> Result<Config, ConfigError> {
    let main_path = dir.join(MAIN_FILE);
    let err = |file: &Path, message: String| ConfigError { file: file.to_path_buf(), message };

    let user_src = read(MAIN_FILE).map_err(|e| err(&main_path, e))?;
    let mut merged: Table = DEFAULT_MAIN.parse().expect("built-in raylauncher.toml is valid");
    let user: Table = user_src.parse().map_err(|e: toml::de::Error| err(&main_path, e.to_string()))?;
    merge(&mut merged, user, "").map_err(|e| err(&main_path, e))?;
    let main: Main = Value::Table(merged).try_into().map_err(|e: toml::de::Error| err(&main_path, e.to_string()))?;
    validate_main(&main).map_err(|e| err(&main_path, e))?;
    let hotkey = crate::hotkey::Combo::parse(&main.hotkey).map_err(|e| err(&main_path, format!("hotkey: {e}")))?;

    let mut commands = Vec::new();
    for name in [COMMANDS_FILE, SYSTEM_FILE] {
        let path = dir.join(name);
        let src = read(name).map_err(|e| err(&path, e))?;
        let file: CommandFile = toml::from_str(&src).map_err(|e| err(&path, e.to_string()))?;
        for (i, c) in file.command.into_iter().enumerate() {
            validate_command(&c).map_err(|e| err(&path, format!("[[command]] #{} ({:?}): {e}", i + 1, c.name)))?;
            if c.enabled {
                commands.push(c);
            }
        }
    }
    Ok(Config { main, hotkey, commands })
}

fn read_optional(p: &Path) -> Result<String, String> {
    match std::fs::read_to_string(p) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Merge `user` onto `base`. Every user key must exist in the defaults with a
/// compatible type, so typos fail with the full key path and the list of valid keys.
fn merge(base: &mut Table, user: Table, prefix: &str) -> Result<(), String> {
    for (k, v) in user {
        let path = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
        let Some(slot) = base.get_mut(&k) else {
            let valid: Vec<&str> = base.keys().map(String::as_str).collect();
            return Err(format!("unknown key `{path}`. Valid keys here: {}", valid.join(", ")));
        };
        match (slot, v) {
            (Value::Table(b), Value::Table(u)) => merge(b, u, &path)?,
            (slot @ Value::Float(_), Value::Integer(i)) => *slot = Value::Float(i as f64),
            (slot, v) if std::mem::discriminant(slot) == std::mem::discriminant(&v) => *slot = v,
            (slot, v) => {
                return Err(format!("`{path}` should be of type {}, found {}", slot.type_str(), v.type_str()));
            }
        }
    }
    Ok(())
}

fn validate_main(m: &Main) -> Result<(), String> {
    let w = &m.window;
    if !(200.0..=3000.0).contains(&w.width) {
        return Err(format!("window.width must be 200-3000, got {}", w.width));
    }
    if !(1..=20).contains(&w.max_results) {
        return Err(format!("window.max_results must be 1-20, got {}", w.max_results));
    }
    if !(8.0..=72.0).contains(&w.font_size) {
        return Err(format!("window.font_size must be 8-72, got {}", w.font_size));
    }
    if !(0.0..=0.8).contains(&w.top_offset) {
        return Err(format!("window.top_offset must be 0-0.8, got {}", w.top_offset));
    }
    if !matches!(w.theme.as_str(), "system" | "dark" | "light") {
        return Err(format!("window.theme must be \"system\", \"dark\" or \"light\", got {:?}", w.theme));
    }
    if m.shell.is_empty() {
        return Err("shell must have at least one element, e.g. [\"/bin/zsh\", \"-lc\"]".into());
    }
    if m.editor.trim().is_empty() {
        return Err("editor must not be empty".into());
    }
    if m.search.history_weight < 0.0 {
        return Err("search.history_weight must be >= 0".into());
    }
    if m.calculator.precision > 15 {
        return Err("calculator.precision must be 0-15".into());
    }
    Ok(())
}

pub const BUILTINS: &[&str] = &["@edit-config", "@reindex", "@quit"];

fn validate_command(c: &Command) -> Result<(), String> {
    if c.name.trim().is_empty() {
        return Err("name must not be empty".into());
    }
    if c.run.trim().is_empty() {
        return Err("run must not be empty".into());
    }
    if c.run.starts_with('@') && !BUILTINS.contains(&c.run.as_str()) {
        return Err(format!("unknown built-in {:?}. Built-ins: {}", c.run, BUILTINS.join(", ")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(main: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("raylauncher-test-{}-{n}", std::process::id()));
        ensure_files(&d).unwrap();
        std::fs::write(d.join(MAIN_FILE), main).unwrap();
        d
    }

    #[test]
    fn defaults_load() {
        let d = dir_with("");
        let c = load(&d).unwrap();
        assert_eq!(c.main.window.max_results, 8);
        assert!(c.commands.iter().any(|c| c.name == "Sleep"));
    }

    #[test]
    fn partial_override_and_int_to_float() {
        let d = dir_with("[window]\nwidth = 900\n");
        let c = load(&d).unwrap();
        assert_eq!(c.main.window.width, 900.0);
        assert_eq!(c.main.window.max_results, 8);
    }

    #[test]
    fn typo_reports_path() {
        let d = dir_with("[window]\nwidht = 900\n");
        let e = load(&d).unwrap_err().message;
        assert!(e.contains("window.widht") && e.contains("width"), "{e}");
    }

    #[test]
    fn wrong_type_reports_path() {
        let d = dir_with("[window]\nmax_results = \"8\"\n");
        let e = load(&d).unwrap_err().message;
        assert!(e.contains("window.max_results"), "{e}");
    }
}
