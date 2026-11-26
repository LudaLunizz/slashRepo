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
    /// `program` with this HOME and a PATH of only this HOME's bin: no agent CLIs, no gh.
    /// (Windows: plus System32 and PowerShell, which the Start Menu entry needs; the rest of the
    /// environment stays, Windows needs SystemRoot and friends.) One of the two places a test may
    /// start a process (see `every_process_starts_in_a_throwaway_home`).
    fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut c = Command::new(program);
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
        c
    }

    fn run(&self, args: &[&str], stdin: Option<&str>, env: &[(&str, &Path)]) -> Output {
        let mut c = self.command(BIN);
        c.args(args);
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

/// The binary with an empty environment: no home folder at all. The other place a test may start
/// a process.
#[cfg(unix)]
fn without_home() -> Command {
    let mut c = Command::new(BIN);
    c.env_clear();
    c
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
    // wrong usage is exit code 2, a failed run is 1
    assert_eq!(h.run(&["--add", "nonsense"], None, &[]).status.code(), Some(2));
    assert_eq!(h.run(&["--bogus"], None, &[]).status.code(), Some(2));
    assert_eq!(read(&cfg), "me/a\n\n[ArtCraft]\nsort = stars\nst/x\n");
    let tabs = h.run(&["--tabs"], None, &[]);
    assert_eq!(String::from_utf8_lossy(&tabs.stdout), "[My repos]\n  me/a\n[ArtCraft]  sort = stars\n  st/x\n");
    assert!(h.run(&["--remove-tab", "ArtCraft"], None, &[]).status.success());
    assert_eq!(read(&cfg), "me/a\n\n");
    assert!(h.run(&["--remove", "me/a"], None, &[]).status.success());
    assert_eq!(read(&cfg), "\n");
}

#[cfg(unix)]
#[test]
fn the_repos_file_follows_xdg_config_home() {
    let h = TempHome::new("xdg");
    let xdg = h.path("elsewhere");
    let out = h.run(&["--add", "me/a"], None, &[("XDG_CONFIG_HOME", &xdg)]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(read(&xdg.join("slashrepo/repos")), "[My repos]\nme/a\n");
    assert!(!h.path(CONFIG).exists());
    // a relative XDG_CONFIG_HOME is ignored, as the spec says
    let out = h.run(&["--add", "me/b"], None, &[("XDG_CONFIG_HOME", Path::new("rel"))]);
    assert!(out.status.success());
    assert_eq!(read(&h.path(CONFIG)), "[My repos]\nme/b\n");
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
fn hook_lets_the_prompt_through_when_the_window_cannot_start() {
    let h = TempHome::new("hook-nostart");
    let gone = h.path("no-such-program");
    let bare = serde_json::json!({"prompt": "/repo"}).to_string();
    let out = h.run(&["hook"], Some(&bare), &[("REPO_STATUS", &gone)]);
    assert!(out.status.success());
    assert!(out.stdout.is_empty(), "a prompt was blocked for a window that is not there: {:?}", out.stdout);
    // a file that is there but cannot run
    #[cfg(unix)]
    {
        let plain = h.path("plain-file");
        std::fs::write(&plain, "not a program").unwrap();
        let out = h.run(&["hook"], Some(&bare), &[("REPO_STATUS", &plain)]);
        assert!(out.status.success() && out.stdout.is_empty(), "{:?}", out.stdout);
    }
}

#[test]
fn hook_ignores_the_phrases_inside_other_words() {
    let h = TempHome::new("hook-words");
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
    for prompt in ["run the monorepo check", "repo checkout is broken", "I recheck the repos nightly"] {
        let input = serde_json::json!({"prompt": prompt}).to_string();
        let out = h.run(&["hook"], Some(&input), &[("REPO_STATUS", &fake)]);
        assert!(out.status.success() && out.stdout.is_empty(), "{prompt}");
    }
    std::thread::sleep(Duration::from_millis(800));
    assert!(!opened.exists(), "a window was started for words inside other words");
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
        ".config/opencode/commands/repo.md",
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
        let opencode = read(&h.path(".config/opencode/commands/repo.md"));
        assert!(opencode.contains(&fwd) && opencode.contains(" open"), "{opencode}");
        assert!(read(&h.path(".agents/skills/repo/SKILL.md")).contains("& '"), "PowerShell line missing");
    }
    assert_eq!(v["theme"], "x");
    assert!(h.command(&installed).arg("--version").output().unwrap().status.success());

    // A machine with none of the agents gets only the window.
    let bare = TempHome::new("install-bare");
    assert!(bare.run(&["install"], None, &[]).status.success());
    assert!(bare.path(own[0]).exists());
    for d in [".claude", ".config/opencode", ".gemini", ".agents", ".codex"] {
        assert!(!bare.path(d).exists(), "bare: made {d}");
    }
}

#[test]
fn install_never_overwrites_a_command_file_that_is_not_ours() {
    let h = TempHome::new("install-own");
    for d in [".claude/commands", ".config/opencode/commands"] {
        std::fs::create_dir_all(h.path(d)).unwrap();
    }
    std::fs::write(h.path(".claude/commands/repo.md"), "my own /repo: list my todos\n").unwrap();
    std::fs::write(h.path(".config/opencode/commands/repo.md"), "# mine\nsummarise the repo\n").unwrap();
    let out = h.run(&["install"], None, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("kept your own"), "{:?}", out.stdout);
    assert_eq!(read(&h.path(".claude/commands/repo.md")), "my own /repo: list my todos\n");
    assert_eq!(read(&h.path(".config/opencode/commands/repo.md")), "# mine\nsummarise the repo\n");
    // the hook does not depend on the command file
    assert!(read(&h.path(".claude/settings.json")).contains(" hook"));
    // a file from an older slashRepo (it calls repo-status) is updated
    std::fs::write(h.path(".config/opencode/commands/repo.md"), "old text: !`repo-status`\n").unwrap();
    assert!(h.run(&["install"], None, &[]).status.success());
    let now = read(&h.path(".config/opencode/commands/repo.md"));
    assert!(now.contains("repo-status") && !now.starts_with("old text"), "{now}");
}

#[test]
fn install_moves_our_old_opencode_command_and_keeps_the_users_own() {
    let h = TempHome::new("install-opencode-move");
    std::fs::create_dir_all(h.path(".config/opencode/command")).unwrap();
    // slashRepo up to 0.6.1 wrote command/repo.md: it moves to commands/, so /repo is listed once
    std::fs::write(h.path(".config/opencode/command/repo.md"), "old text: !`repo-status`\n").unwrap();
    let out = h.run(&["install"], None, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("moved from"), "{:?}", out.stdout);
    assert!(!h.path(".config/opencode/command/repo.md").exists());
    assert!(read(&h.path(".config/opencode/commands/repo.md")).contains("repo-status"));
    // a command/repo.md of the user's own stays where it is
    std::fs::write(h.path(".config/opencode/command/repo.md"), "# mine\n").unwrap();
    assert!(h.run(&["install"], None, &[]).status.success());
    assert_eq!(read(&h.path(".config/opencode/command/repo.md")), "# mine\n");
}

#[test]
fn edits_at_the_same_time_all_land() {
    let h = TempHome::new("edit-lock");
    let list = h.path(CONFIG);
    std::fs::create_dir_all(list.parent().unwrap()).unwrap();
    std::fs::write(&list, "me/start\n").unwrap();
    let names: Vec<String> = (0..12).map(|i| format!("me/r{i}")).collect();
    std::thread::scope(|s| {
        for n in &names {
            let h = &h;
            s.spawn(move || assert!(h.run(&["--add", n], None, &[]).status.success()));
        }
    });
    let text = read(&list);
    for n in names.iter().map(String::as_str).chain(["me/start"]) {
        assert!(text.lines().any(|l| l.trim() == n), "lost {n}:\n{text}");
    }
}

#[test]
fn one_failing_agent_does_not_stop_the_others() {
    let h = TempHome::new("install-partial");
    std::fs::create_dir_all(h.path(".claude")).unwrap();
    std::fs::create_dir_all(h.path(".config/opencode")).unwrap();
    std::fs::write(h.path(".claude/settings.json"), "this is not json").unwrap();
    let out = h.run(&["install"], None, &[]);
    assert!(!out.status.success());
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("Claude Code: FAILED") && stdout.contains("opencode: /repo"), "{stdout}");
    assert!(stderr.contains("1 of 2 steps failed"), "{stderr}");
    assert!(h.path(".config/opencode/commands/repo.md").exists());
    // nothing of Claude Code was written: not the command file, and settings.json is as it was
    assert!(!h.path(".claude/commands/repo.md").exists());
    assert_eq!(read(&h.path(".claude/settings.json")), "this is not json");
    assert!(!h.path(".claude/settings.json.bak-slashrepo").exists());
}

#[cfg(unix)]
#[test]
fn install_keeps_the_mode_and_the_symlink_of_settings_json() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let h = TempHome::new("install-mode");
    std::fs::create_dir_all(h.path(".claude")).unwrap();
    std::fs::create_dir_all(h.path("dotfiles")).unwrap();
    let real = h.path("dotfiles/settings.json");
    std::fs::write(&real, "{\"theme\": \"x\"}").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = h.path(".claude/settings.json");
    symlink(&real, &link).unwrap();
    assert!(h.run(&["install"], None, &[]).status.success());
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link was replaced");
    assert!(read(&real).contains(" hook") && read(&real).contains("\"theme\""));
    assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
    // the backup is as private as the original and holds the state before the change
    let backup = h.path(".claude/settings.json.bak-slashrepo");
    assert_eq!(std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(read(&backup), "{\"theme\": \"x\"}");
    // a second install changes nothing and leaves that backup alone
    assert!(h.run(&["install"], None, &[]).status.success());
    assert_eq!(read(&backup), "{\"theme\": \"x\"}");
    // after the user edits the file (hook gone, new key), the next change gets a fresh backup
    std::fs::write(&real, "{\"theme\": \"y\", \"new\": 1}").unwrap();
    assert!(h.run(&["install"], None, &[]).status.success());
    assert_eq!(read(&backup), "{\"theme\": \"y\", \"new\": 1}");
    assert!(read(&real).contains("\"new\"") && read(&real).contains(" hook"));
}

#[cfg(unix)]
#[test]
fn install_without_a_home_folder_stops_before_writing() {
    let out = without_home().arg("install").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("HOME is not set"), "{:?}", out.stderr);
}

#[test]
fn uninstall_undoes_install_and_keeps_what_is_not_ours() {
    let h = TempHome::new("uninstall");
    for d in [".claude", ".config/opencode", ".codex", ".gemini"] {
        std::fs::create_dir_all(h.path(d)).unwrap();
    }
    let other = serde_json::json!({"type": "command", "command": "echo hi"});
    std::fs::write(
        h.path(".claude/settings.json"),
        serde_json::json!({"theme": "x", "hooks": {
            "UserPromptSubmit": [{"hooks": [other]}],
            "Stop": [{"hooks": [other]}]}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(h.path(".gemini/settings.json"), "{}").unwrap();
    std::fs::create_dir_all(h.path(".gemini/commands")).unwrap();
    std::fs::write(h.path(".gemini/commands/repo.toml"), "mine = true\n").unwrap();
    let list = h.path(CONFIG);
    std::fs::create_dir_all(list.parent().unwrap()).unwrap();
    std::fs::write(&list, "me/a\n").unwrap();
    assert!(h.run(&["install"], None, &[]).status.success());
    assert!(read(&h.path(".claude/settings.json")).contains(" hook"));
    // an opencode command of ours left in the old folder (from before 0.6.2) goes too
    std::fs::create_dir_all(h.path(".config/opencode/command")).unwrap();
    std::fs::write(h.path(".config/opencode/command/repo.md"), "!`repo-status`\n").unwrap();

    let out = h.run(&["uninstall"], None, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("hook removed") && said.contains("kept") && said.contains("not ours"), "{said}");
    // what install made is gone
    for f in [
        ".claude/commands/repo.md",
        ".config/opencode/commands/repo.md",
        ".config/opencode/command/repo.md",
        ".agents/skills/repo/SKILL.md",
        if cfg!(windows) { "AppData/Local/Programs/slashRepo/repo-status.exe" } else { ".local/bin/repo-status" },
    ] {
        assert!(!h.path(f).exists(), "still there: {f}");
    }
    #[cfg(unix)]
    for f in [
        ".local/share/applications/io.github.ludalunizz.SlashRepo.desktop",
        ".local/share/icons/hicolor/scalable/apps/io.github.ludalunizz.SlashRepo.svg",
    ] {
        assert!(!h.path(f).exists(), "still there: {f}");
    }
    #[cfg(windows)]
    assert!(!h.path("AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Repo Status.lnk").exists());
    // what is not ours is not touched: the user's own command, the list, the other hooks, the theme
    assert_eq!(read(&h.path(".gemini/commands/repo.toml")), "mine = true\n");
    assert_eq!(read(&list), "me/a\n");
    let v: serde_json::Value = serde_json::from_str(&read(&h.path(".claude/settings.json"))).unwrap();
    assert_eq!(v["theme"], "x");
    assert_eq!(v["hooks"]["UserPromptSubmit"][0]["hooks"], serde_json::json!([other]));
    assert_eq!(v["hooks"]["Stop"][0]["hooks"], serde_json::json!([other]));
    // the backup is the settings.json from just before the removal
    assert!(read(&h.path(".claude/settings.json.bak-slashrepo")).contains("repo-status"));

    // twice: nothing left to do, and still success
    let again = h.run(&["uninstall"], None, &[]);
    assert!(again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("nothing to remove")
            || String::from_utf8_lossy(&again.stdout).contains("not ours")
    );
}

#[test]
fn uninstall_leaves_a_settings_file_that_held_only_our_hook_empty_but_valid() {
    let h = TempHome::new("uninstall-only");
    std::fs::create_dir_all(h.path(".claude")).unwrap();
    assert!(h.run(&["install"], None, &[]).status.success());
    assert!(h.run(&["uninstall"], None, &[]).status.success());
    let v: serde_json::Value = serde_json::from_str(&read(&h.path(".claude/settings.json"))).unwrap();
    assert_eq!(v, serde_json::json!({}));
    // a settings.json that is not JSON is left as it is, and the other steps still run
    std::fs::write(h.path(".claude/settings.json"), "broken").unwrap();
    let out = h.run(&["uninstall"], None, &[]);
    assert!(!out.status.success());
    assert_eq!(read(&h.path(".claude/settings.json")), "broken");
}

#[cfg(unix)]
#[test]
fn uninstall_removes_the_trigger_units() {
    let h = TempHome::new("uninstall-units");
    let dir = h.path(".config/systemd/user");
    std::fs::create_dir_all(&dir).unwrap();
    for u in ["slashrepo-trigger.path", "slashrepo-trigger.service", "other.service"] {
        std::fs::write(dir.join(u), "[Unit]\n").unwrap();
    }
    let out = h.run(&["uninstall"], None, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!dir.join("slashrepo-trigger.path").exists() && !dir.join("slashrepo-trigger.service").exists());
    assert!(dir.join("other.service").exists(), "somebody else's unit");
}

/// A test that starts the binary with the real HOME could change the user's settings.json or repos
/// list. Every process start in this file must sit in one of the two constructors that isolate it;
/// the scan must find both, so a scan that reads nothing cannot pass.
#[test]
fn every_process_starts_in_a_throwaway_home() {
    let source = include_str!("cli.rs");
    let needle = concat!("Command", "::new(");
    let allowed = ["command", "without_home"];
    let (mut current, mut seen) = ("", Vec::new());
    for (n, line) in source.lines().enumerate() {
        if let Some(rest) = line.trim_start().split("fn ").nth(1).filter(|_| line.contains("fn ")) {
            current = rest.split(['(', '<']).next().unwrap_or("");
        }
        if line.contains(needle) {
            assert!(
                allowed.contains(&current),
                "line {}: a process started in fn {current}, not in {allowed:?}",
                n + 1
            );
            seen.push(current);
        }
    }
    for f in allowed {
        assert!(seen.contains(&f), "the scan did not find the constructor {f}");
    }
}
