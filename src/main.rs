mod app;
mod calc;
mod catalog;
mod config;
mod hotkey;
mod ui;
mod watch;

use std::path::PathBuf;
use std::process::{Command, exit};

const LABEL: &str = "local.raylauncher";

const USAGE: &str = "\
raylauncher: keyboard launcher for macOS (apps, commands, calculator)

Usage:
  raylauncher              Run raylauncher in the foreground
  raylauncher toggle       Show or hide the window of the running raylauncher
  raylauncher check        Validate the config files; exit 1 on error
  raylauncher query TEXT   Print the results raylauncher would show for TEXT
  raylauncher paths        Print config, state, and log locations
  raylauncher install      Start at login (LaunchAgent). Copies the binary to ~/.local/bin
                           unless it was installed by Homebrew
  raylauncher --version    Print the version
  raylauncher uninstall    Remove the LaunchAgent and stop raylauncher

Config lives in ~/.config/raylauncher (override with RAYLAUNCHER_CONFIG_DIR).
Edits apply as soon as a file is saved. See AGENTS.md in that folder.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => app::run(),
        Some("check") => check(),
        Some("toggle") => ui::post_toggle(),
        Some("query") => query(&args[1..].join(" ")),
        Some("paths") => paths(),
        Some("install") => install(),
        Some("uninstall") => uninstall(),
        Some("-h" | "--help" | "help") => println!("{USAGE}"),
        Some("-V" | "--version" | "version") => println!("raylauncher {}", env!("CARGO_PKG_VERSION")),
        Some(other) => {
            eprintln!("unknown command {other:?}\n\n{USAGE}");
            exit(2);
        }
    }
}

fn check() {
    let dir = config::config_dir();
    if let Err(e) = config::ensure_files(&dir) {
        eprintln!("cannot create {}: {e}", dir.display());
        exit(1);
    }
    match config::load(&dir) {
        Ok(c) => {
            let apps = catalog::scan_apps(&c.main.search);
            println!(
                "ok: {} (hotkey {}, {} apps, {} commands)",
                dir.display(),
                c.main.hotkey,
                apps.len(),
                c.commands.len()
            );
        }
        Err(e) => {
            eprintln!("error: {e}");
            exit(1);
        }
    }
}

fn query(q: &str) {
    let dir = config::config_dir();
    let (cfg, err) = match config::load(&dir) {
        Ok(c) => (c, None),
        Err(e) => (config::builtin(), Some(e)),
    };
    let mut cat = catalog::Catalog::new(&cfg);
    cat.refresh_apps(&cfg);
    let history = catalog::History::load(config::state_dir().join("history.tsv"));
    let limit = cfg.main.window.max_results;
    for it in app::compute_results(q, &cfg, err.as_ref(), &mut cat, &history, limit) {
        println!("{}\t{}\t{:?}", it.title, it.subtitle, it.action);
    }
}

fn paths() {
    let state = config::state_dir();
    println!("config   {}", config::config_dir().display());
    println!("state    {}", state.display());
    println!("history  {}", state.join("history.tsv").display());
    println!("log      {}", state.join("raylauncher.log").display());
    println!("agent    {}", plist_path().display());
}

fn plist_path() -> PathBuf {
    config::home().join(format!("Library/LaunchAgents/{LABEL}.plist"))
}

unsafe extern "C" {
    fn getuid() -> u32;
}

fn launchctl(args: &[&str]) -> bool {
    Command::new("/bin/launchctl").args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// For a Homebrew install (`<prefix>/Cellar/raylauncher/<version>/bin/raylauncher`), the
/// version-independent `<prefix>/opt/raylauncher/bin/raylauncher`, so `brew upgrade`
/// updates the binary the LaunchAgent runs.
fn homebrew_path(exe: &std::path::Path) -> Option<PathBuf> {
    let s = exe.to_str()?;
    let (prefix, _) = s.split_once("/Cellar/raylauncher/")?;
    let opt = PathBuf::from(prefix).join("opt/raylauncher/bin/raylauncher");
    opt.exists().then_some(opt)
}

fn install() {
    let current = std::env::current_exe().and_then(std::fs::canonicalize).expect("cannot locate own binary");
    let domain = format!("gui/{}", unsafe { getuid() });
    stop_agent(&domain);
    let target = match homebrew_path(&current) {
        Some(p) => p,
        None => {
            let bin_dir = config::home().join(".local/bin");
            let target = bin_dir.join("raylauncher");
            if std::fs::canonicalize(&target).ok() != Some(current.clone()) {
                std::fs::create_dir_all(&bin_dir).expect("create ~/.local/bin");
                // Remove first: overwriting a running binary in place can invalidate its code signature.
                let _ = std::fs::remove_file(&target);
                std::fs::copy(&current, &target).expect("copy binary");
            }
            target
        }
    };
    let state = config::state_dir();
    std::fs::create_dir_all(&state).expect("create state dir");
    let log = state.join("raylauncher.log");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{bin}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        bin = target.display(),
        log = log.display()
    );
    let p = plist_path();
    std::fs::create_dir_all(p.parent().unwrap()).expect("create LaunchAgents dir");
    std::fs::write(&p, plist).expect("write plist");
    if launchctl(&["bootstrap", &domain, &p.to_string_lossy()]) {
        println!("installed {} and started {LABEL}", target.display());
        println!("log: {}", log.display());
    } else {
        eprintln!("wrote {} but `launchctl bootstrap` failed; run it manually", p.display());
        exit(1);
    }
}

/// `launchctl bootout` returns before the process exits; bootstrapping again too soon
/// fails with an I/O error. Wait up to 5 s for the service to disappear.
fn stop_agent(domain: &str) {
    let target = format!("{domain}/{LABEL}");
    if !launchctl(&["bootout", &target]) {
        return; // not loaded
    }
    for _ in 0..50 {
        if !launchctl(&["print", &target]) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn uninstall() {
    let domain = format!("gui/{}", unsafe { getuid() });
    stop_agent(&domain);
    let _ = std::fs::remove_file(plist_path());
    println!("removed {LABEL}. Config in {} was left in place.", config::config_dir().display());
}

#[cfg(test)]
mod tests {
    use super::homebrew_path;

    #[test]
    fn homebrew_opt_path() {
        let prefix = std::env::temp_dir().join(format!("raylauncher-brew-{}", std::process::id()));
        let opt = prefix.join("opt/raylauncher/bin/raylauncher");
        std::fs::create_dir_all(opt.parent().unwrap()).unwrap();
        std::fs::write(&opt, "").unwrap();
        let cellar = prefix.join("Cellar/raylauncher/0.1.0/bin/raylauncher");
        assert_eq!(homebrew_path(&cellar), Some(opt));
        assert_eq!(homebrew_path(std::path::Path::new("/Users/x/.local/bin/raylauncher")), None);
    }
}
