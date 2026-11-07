//! Searchable items (apps + commands), optional launch history, and ranking with nucleo, a
//! Rust implementation of fzf's matching algorithm.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32String};

use crate::config::{Command, Config, Search, expand_tilde};

#[derive(Debug, Clone, PartialEq)]
pub enum Icon {
    None,
    File(PathBuf),
    Symbol(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    OpenApp(PathBuf),
    Run { cmd: String, confirm: bool },
    Builtin(String),
    Copy(String),
    EditFile(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Item {
    /// Stable identity used as the history key: the app path, or `cmd:<name>`.
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub icon: Icon,
    pub action: Action,
    hay_title: Utf32String,
    hay_keywords: Option<Utf32String>,
}

impl Item {
    fn new(id: String, title: String, subtitle: String, icon: Icon, action: Action, keywords: &str) -> Item {
        let hay_title = Utf32String::from(title.as_str());
        let hay_keywords = (!keywords.trim().is_empty()).then(|| Utf32String::from(keywords));
        Item { id, title, subtitle, icon, action, hay_title, hay_keywords }
    }

    /// A one-off result (calculator answer, config error) that is never in the index.
    pub fn transient(title: String, subtitle: String, icon: Icon, action: Action) -> Item {
        Item::new(String::new(), title, subtitle, icon, action, "")
    }
}

/// Apps from `search.extra_apps` first, then every `.app` found in `search.app_dirs`.
/// When two apps share a name, the earlier one wins.
pub fn scan_apps(s: &Search) -> Vec<Item> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |path: &Path| {
        let Some(name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { return };
        if s.exclude.iter().any(|e| e == &name) || !seen.insert(name.clone()) {
            return;
        }
        let parent = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
        out.push(Item::new(
            path.display().to_string(),
            name,
            parent,
            Icon::File(path.to_path_buf()),
            Action::OpenApp(path.to_path_buf()),
            "",
        ));
    };
    for a in &s.extra_apps {
        let p = expand_tilde(a);
        if p.exists() {
            add(&p);
        }
    }
    for d in &s.app_dirs {
        walk(&expand_tilde(d), s.scan_depth, &mut add);
    }
    out
}

fn walk(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if name.ends_with(".app") {
            f(&p);
        } else if depth > 0 && p.is_dir() {
            walk(&p, depth - 1, f);
        }
    }
}

pub fn command_items(cmds: &[Command]) -> Vec<Item> {
    cmds.iter()
        .map(|c| {
            let icon = match c.icon.as_deref() {
                Some(s) if s.starts_with("sf:") => Icon::Symbol(s[3..].to_string()),
                Some(s) if !s.is_empty() => Icon::File(expand_tilde(s)),
                _ => Icon::None,
            };
            let action = if c.run.starts_with('@') {
                Action::Builtin(c.run.clone())
            } else {
                Action::Run { cmd: c.run.clone(), confirm: c.confirm }
            };
            let subtitle = c.subtitle.clone().unwrap_or_else(|| c.run.clone());
            Item::new(format!("cmd:{}", c.name), c.name.clone(), subtitle, icon, action, &c.keywords)
        })
        .collect()
}

/// Launch counts, stored as `count<TAB>id` lines so people and agents can read or edit
/// them. Only used when `search.history = true`.
#[derive(Default)]
pub struct History {
    path: Option<PathBuf>,
    counts: HashMap<String, u32>,
}

impl History {
    pub fn load(path: PathBuf) -> History {
        let mut counts = HashMap::new();
        if let Ok(s) = std::fs::read_to_string(&path) {
            for line in s.lines() {
                if let Some((n, id)) = line.split_once('\t') {
                    if let Ok(n) = n.trim().parse() {
                        counts.insert(id.to_string(), n);
                    }
                }
            }
        }
        History { path: Some(path), counts }
    }

    pub fn get(&self, id: &str) -> u32 {
        self.counts.get(id).copied().unwrap_or(0)
    }

    pub fn bump(&mut self, id: &str) {
        if id.is_empty() {
            return;
        }
        *self.counts.entry(id.to_string()).or_default() += 1;
        if let Some(p) = &self.path {
            let mut rows: Vec<_> = self.counts.iter().collect();
            rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let body: String = rows.iter().map(|(id, n)| format!("{n}\t{id}\n")).collect();
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, body);
        }
    }
}

fn caseless(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().flat_map(char::to_lowercase)
}

pub struct Catalog {
    pub apps: Vec<Item>,
    pub commands: Vec<Item>,
    pub apps_dirty: bool,
    matcher: Matcher,
}

impl Catalog {
    pub fn new(cfg: &Config) -> Catalog {
        let mut c = Catalog { apps: Vec::new(), commands: Vec::new(), apps_dirty: true, matcher: Matcher::default() };
        c.apply_config(cfg);
        c
    }

    pub fn apply_config(&mut self, cfg: &Config) {
        let mut mc = nucleo_matcher::Config::DEFAULT;
        mc.prefer_prefix = cfg.main.search.prefer_prefix;
        self.matcher = Matcher::new(mc);
        self.commands = command_items(&cfg.commands);
        self.apps_dirty = true;
    }

    pub fn refresh_apps(&mut self, cfg: &Config) {
        if self.apps_dirty {
            self.apps = scan_apps(&cfg.main.search);
            self.apps_dirty = false;
        }
    }

    /// Rank items for `query`. With history on, launch counts add a bonus and an empty
    /// query lists the most-launched items (when `show_recent` is set); with history off,
    /// an empty query returns nothing. Ties go to the shorter, then earlier title.
    pub fn search(&mut self, query: &str, cfg: &Config, history: &History, limit: usize) -> Vec<Item> {
        let s = &cfg.main.search;
        let q = query.trim();
        let weight = if s.history { s.history_weight } else { 0.0 };
        let boost = |id: &str| weight * (1.0 + history.get(id) as f64).ln();

        let mut scored: Vec<(f64, &Item)> = Vec::new();
        if q.is_empty() {
            if !(s.history && s.show_recent) {
                return Vec::new();
            }
            for it in self.apps.iter().chain(self.commands.iter()) {
                let n = history.get(&it.id);
                if n > 0 {
                    scored.push((n as f64, it));
                }
            }
        } else {
            let pat = Pattern::parse(q, CaseMatching::Smart, Normalization::Smart);
            let matcher = &mut self.matcher;
            for it in self.apps.iter().chain(self.commands.iter()) {
                let t = pat.score(it.hay_title.slice(..), matcher);
                // Keyword hits count, at a discount, so the visible name wins ties.
                let k = it.hay_keywords.as_ref().and_then(|k| pat.score(k.slice(..), matcher)).map(|v| v * 3 / 4);
                if let Some(best) = t.max(k) {
                    let pinned = s.boost.get(&it.title).copied().unwrap_or(0.0);
                    scored.push((best as f64 + pinned + boost(&it.id), it));
                }
            }
        }
        scored.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then(a.1.title.chars().count().cmp(&b.1.title.chars().count()))
                .then_with(|| caseless(&a.1.title).cmp(caseless(&b.1.title)))
                .then_with(|| a.1.title.cmp(&b.1.title))
        });
        scored.into_iter().take(limit).map(|(_, it)| it.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(history: bool) -> Config {
        let d = std::env::temp_dir().join(format!("raylauncher-cat-{}", std::process::id()));
        crate::config::ensure_files(&d).unwrap();
        let mut c = crate::config::load(&d).unwrap();
        c.main.search.history = history;
        c
    }

    fn app(name: &str) -> Item {
        Item::new(name.into(), name.into(), String::new(), Icon::None, Action::OpenApp(name.into()), "")
    }

    #[test]
    fn ranks_prefix_and_keywords() {
        let cfg = cfg(false);
        let mut c = Catalog::new(&cfg);
        c.apps = ["Safari", "System Settings", "Slack", "Font Book"].map(app).to_vec();
        c.apps_dirty = false;
        let h = History::default();
        assert_eq!(c.search("saf", &cfg, &h, 5)[0].title, "Safari");
        assert_eq!(c.search("sys set", &cfg, &h, 5)[0].title, "System Settings");
        assert_eq!(c.search("reboot", &cfg, &h, 5)[0].title, "Restart");
        let r = c.search("s !slack", &cfg, &h, 10);
        assert!(r.iter().all(|i| i.title != "Slack"));
    }

    #[test]
    fn ties_ignore_case_and_boost_wins() {
        let mut cfg = cfg(false);
        cfg.main.search.boost.clear();
        let mut c = Catalog::new(&cfg);
        c.apps = ["FindMy", "Finder", "Firefox"].map(app).to_vec();
        c.commands.clear();
        c.apps_dirty = false;
        let h = History::default();
        // equal score and length: "finder" < "findmy" ignoring case
        assert_eq!(c.search("f", &cfg, &h, 5)[0].title, "Finder");
        cfg.main.search.boost.insert("Firefox".into(), 20.0);
        assert_eq!(c.search("f", &cfg, &h, 5)[0].title, "Firefox");
        cfg.main.search.boost.insert("Firefox".into(), -20.0);
        assert_eq!(c.search("f", &cfg, &h, 5).last().unwrap().title, "Firefox");
    }

    #[test]
    fn history_only_applies_when_enabled() {
        let mut h = History::default();
        let off = cfg(false);
        let mut c = Catalog::new(&off);
        c.apps = ["Slack", "Safari"].map(app).to_vec();
        c.apps_dirty = false;
        let before = c.search("s", &off, &h, 5);
        for _ in 0..20 {
            h.bump(&before[1].id);
        }
        assert_eq!(c.search("s", &off, &h, 5)[0].title, before[0].title);
        assert!(c.search("", &off, &h, 5).is_empty());

        let on = cfg(true);
        assert_eq!(c.search("s", &on, &h, 5)[0].title, before[1].title);
        assert_eq!(c.search("", &on, &h, 5)[0].title, before[1].title);
    }
}
