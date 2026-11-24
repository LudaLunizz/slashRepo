//! The built binary in a throwaway HOME: the list flags, the prompt hook and the installer.
//! Runs on Linux and Windows (on Windows the home is USERPROFILE and the list is under APPDATA).

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_repo-status");

struct TempHome(PathBuf);

impl TempHome {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("slashrepo-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("bin")).unwrap();
        // setsid and sh only: agent CLIs on the real PATH must not count as installed
        #[cfg(unix)]
        for tool in ["setsid", "sh", "touch"] {
            let found = ["/usr/bin", "/bin"].iter().map(|d| Path::new(d).join(tool)).find(|t| t.exists());
            std::os::unix::fs::symlink(found.unwrap(), p.join("bin").join(tool)).unwrap();
        }
        TempHome(p)
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
    /// PATH has only this HOME's bin: no agent CLIs, no gh. (Windows: plus System32 and
    /// PowerShell, which the Start Menu entry needs; the rest of the environment stays, Windows
    /// needs SystemRoot and friends.)
    fn run(&self, args: &[&str], stdin: Option<&str>, env: &[(&str, &Path)]) -> Output {
        let mut c = Command::new(BIN);
        c.args(args);
        #[cfg(unix)]
        c.env_clear().env("HOME", &self.0).env("PATH", self.path("bin"));
        #[cfg(windows)]
        {
            let sys = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
            let path = std::env::join_paths([self.path("bin"), sys.clone(), sys.join("WindowsPowerShell/v1.0")]);
            c.env("USERPROFILE", &self.0)
                .env("APPDATA", self.path("AppData/Roaming"))
                .env("LOCALAPPDATA", self.path("AppData/Local"))
                .env("PATH", path.unwrap());
        }
        for (k, v) in env {
            c.env(k, v);
        }
        c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        if let Some(s) = stdin {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        }
        drop(child.stdin.take());
        child.wait_with_output().unwrap()
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The repos file inside a TempHome.
const CONFIG: &str = if cfg!(windows) { "AppData/Roaming/slashRepo/repos" } else { ".config/slashrepo/repos" };

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

#[test]
fn flags_edit_the_real_file() {
    let h = TempHome::new("flags");
    let cfg = h.path(CONFIG);
    std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    std::fs::write(&cfg, "me/a\n").unwrap();
    assert!(h.run(&["--add", "https://github.com/st/x", "ArtCraft"], None, &[]).status.success());
    assert!(h.run(&["--sort", "ArtCraft", "stars"], None, &[]).status.success());
    assert!(!h.run(&["--add", "nonsense"], None, &[]).status.success());
    assert!(!h.run(&["--bogus"], None, &[]).status.success());
    assert_eq!(read(&cfg), "me/a\n\n[ArtCraft]\nsort = stars\nst/x\n");
    let tabs = h.run(&["--tabs"], None, &[]);
    assert_eq!(String::from_utf8_lossy(&tabs.stdout), "[My repos]\n  me/a\n[ArtCraft]  sort = stars\n  st/x\n");
    assert!(h.run(&["--remove-tab", "ArtCraft"], None, &[]).status.success());
    assert_eq!(read(&cfg), "me/a\n\n");
    assert!(h.run(&["--remove", "me/a"], None, &[]).status.success());
    assert_eq!(read(&cfg), "\n");
}

#[test]
fn a_repos_file_that_cannot_be_read_is_never_overwritten() {
    let h = TempHome::new("unreadable");
    let cfg = h.path(CONFIG);
    std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    // a hand-edited file with a Latin-1 comment: there, but not UTF-8
    let mine: &[u8] = b"me/a # caf\xe9\n[Friends]\nst/x\n";
    std::fs::write(&cfg, mine).unwrap();
    for args in
        [&["--add", "me/new"][..], &["--remove", "me/a"], &["--remove-tab", "Friends"], &["--sort", "Friends", "stars"]]
    {
        let out = h.run(args, None, &[]);
        assert!(!out.status.success(), "{args:?} must fail");
        assert!(String::from_utf8_lossy(&out.stderr).contains("left untouched"), "{args:?}: {:?}", out.stderr);
        assert_eq!(std::fs::read(&cfg).unwrap(), mine, "{args:?} changed the file");
    }
    // reading says why instead of listing a made-up default
    for args in [&["--tabs"][..], &["--text"]] {
        let out = h.run(args, None, &[]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("Could not read"));
        assert!(out.stdout.is_empty());
    }
}

#[test]
fn hook_opens_the_window_and_blocks_only_the_bare_command() {
    let h = TempHome::new("hook");
    let opened = h.path("opened");
    #[cfg(unix)]
    let fake = {
        let fake = h.path("fake");
        std::fs::write(&fake, format!("#!/bin/sh\ntouch '{}'\n", opened.display())).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        fake
    };
    #[cfg(windows)]
    let fake = {
        let fake = h.path("fake.bat");
        std::fs::write(&fake, format!("@echo off\r\ntype nul > \"{}\"\r\n", opened.display())).unwrap();
        fake
    };
    let cases = [
        (true, true, "/repo"),
        (true, true, "please check the repos"),
        (true, false, "/repo and then fix the tab bug"),
        (false, false, "fix the bug in repository.py"),
    ];
    for (want_open, want_block, prompt) in cases {
        let _ = std::fs::remove_file(&opened);
        let input = serde_json::json!({"prompt": prompt}).to_string();
        let out = h.run(&["hook"], Some(&input), &[("REPO_STATUS", &fake)]);
        assert!(out.status.success(), "{prompt}");
        let blocked = String::from_utf8_lossy(&out.stdout).contains(r#""decision":"block""#);
        let start = Instant::now();
        while want_open && !opened.exists() && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        if !want_open {
            std::thread::sleep(Duration::from_millis(300));
        }
        assert_eq!((opened.exists(), blocked), (want_open, want_block), "{prompt}");
    }
    // not JSON at all: still exit 0, nothing opened
    let _ = std::fs::remove_file(&opened);
    let out = h.run(&["hook"], Some("garbage"), &[("REPO_STATUS", &fake)]);
    assert!(out.status.success() && out.stdout.is_empty());
}

#[test]
fn install_sets_up_each_agent_present_and_only_those() {
    let h = TempHome::new("install");
    for d in [".claude", ".config/opencode", ".codex", ".gemini/antigravity", ".local/bin"] {
        std::fs::create_dir_all(h.path(d)).unwrap();
    }
    std::fs::write(
        h.path(".claude/settings.json"),
        serde_json::json!({"theme": "x", "hooks": {"UserPromptSubmit": [{"hooks": [
            {"type": "command", "command": h.path(".local/bin/repo-status-hook").to_string_lossy(), "timeout": 5}
        ]}]}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(h.path(".local/bin/repo-status-hook"), "#!/bin/sh\n# open the Repo Status window\n").unwrap();
    std::fs::write(h.path(".gemini/settings.json"), "{}").unwrap();
    std::fs::create_dir_all(h.path(".local/share/applications")).unwrap();
    std::fs::write(h.path(".local/share/applications/repo-status.desktop"), "[Desktop Entry]\n").unwrap();
    for _ in 0..2 {
        // twice: the second run changes nothing
        let out = h.run(&["install"], None, &[]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
    #[cfg(unix)]
    let own = [
        ".local/bin/repo-status",
        ".local/share/applications/io.github.ludalunizz.SlashRepo.desktop",
        ".local/share/icons/hicolor/scalable/apps/io.github.ludalunizz.SlashRepo.svg",
    ];
    #[cfg(windows)]
    let own = [
        "AppData/Local/Programs/slashRepo/repo-status.exe",
        "AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Repo Status.lnk",
    ];
    for f in own.into_iter().chain([
        ".claude/commands/repo.md",
        ".config/opencode/command/repo.md",
        ".gemini/commands/repo.toml",
        ".agents/skills/repo/SKILL.md",
        ".gemini/config/skills/repo/SKILL.md",
        ".claude/settings.json.bak-slashrepo",
    ]) {
        assert!(h.path(f).exists(), "missing {f}");
    }
    assert!(!h.path(".local/bin/repo-status-hook").exists(), "old shell hook left behind");
    #[cfg(unix)]
    assert!(!h.path(".local/share/applications/repo-status.desktop").exists(), "old desktop file left behind");
    let v: serde_json::Value = serde_json::from_str(&read(&h.path(".claude/settings.json"))).unwrap();
    let list = v["hooks"]["UserPromptSubmit"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{v}");
    let installed = h.path(own[0]);
    #[cfg(unix)]
    assert_eq!(list[0]["hooks"][0]["command"], format!("{} hook", installed.display()).as_str());
    #[cfg(windows)]
    {
        // forward slashes (Git Bash runs Claude Code's hooks), quoted when the path needs it
        let cmd = list[0]["hooks"][0]["command"].as_str().unwrap();
        let fwd = installed.to_string_lossy().replace('\\', "/");
        assert!(cmd.contains(&fwd) && cmd.ends_with(" hook") && !cmd.contains('\\'), "{cmd}");
        let opencode = read(&h.path(".config/opencode/command/repo.md"));
        assert!(opencode.contains(&fwd) && opencode.contains(" open"), "{opencode}");
        assert!(read(&h.path(".agents/skills/repo/SKILL.md")).contains("& '"), "PowerShell line missing");
    }
    assert_eq!(v["theme"], "x");
    assert!(Command::new(&installed).arg("--version").output().unwrap().status.success());

    // A machine with none of the agents gets only the window.
    let bare = TempHome::new("install-bare");
    assert!(bare.run(&["install"], None, &[]).status.success());
    assert!(bare.path(own[0]).exists());
    for d in [".claude", ".config/opencode", ".gemini", ".agents", ".codex"] {
        assert!(!bare.path(d).exists(), "bare: made {d}");
    }
}
