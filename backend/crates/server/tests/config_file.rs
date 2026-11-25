//! The binary's settings layers, end to end: `tokuly config show` and the
//! start-up of a role with a `tokuly.toml`, the environment, or both.
use std::{path::PathBuf, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_tokuly");

/// A command without any `TOKULY_*` variable of the test environment.
fn tokuly(args: &[&str]) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.args(args);
    for (name, _) in std::env::vars() {
        if name.starts_with("TOKULY_") {
            cmd.env_remove(name);
        }
    }
    cmd
}

fn file(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tokuly-config-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn run(cmd: &mut Command) -> (bool, String, String) {
    let out = cmd.output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn environment_only_deployments_read_as_before() {
    let (ok, out, err) = run(tokuly(&["config", "show"]).env("TOKULY_MEDIA_MAX_VIDEO_HOURS", "4").env("TOKULY_SECRET", "s".repeat(40)));
    assert!(ok, "{err}");
    assert!(out.contains("[media]\nmax_video_hours = 4  # from 環境変数"), "{out}");
    assert!(!out.contains("ssssssss"), "secrets are not shown: {out}");
}

#[test]
fn the_environment_wins_over_the_file() {
    let path = file("both.toml", "[media]\nmax_video_hours = 5\n\n[core]\nmail_from = \"Tokuly <hi@example.com>\"\n");
    let (ok, out, err) = run(tokuly(&["config", "show"]).env("TOKULY_CONFIG", &path));
    assert!(ok, "{err}");
    assert!(out.contains("max_video_hours = 5  # from tokuly.toml"), "{out}");
    assert!(out.contains("mail_from = \"Tokuly <hi@example.com>\"  # from tokuly.toml"), "{out}");
    let (ok, out, _) = run(tokuly(&["config", "show"]).env("TOKULY_CONFIG", &path).env("TOKULY_MEDIA_MAX_VIDEO_HOURS", "4"));
    assert!(ok && out.contains("max_video_hours = 4  # from 環境変数"), "{out}");
    // A command-line flag wins over both.
    let (ok, out, _) = run(tokuly(&["--max-video-hours", "6", "config", "show"]).env("TOKULY_CONFIG", &path).env("TOKULY_MEDIA_MAX_VIDEO_HOURS", "4"));
    assert!(ok && out.contains("max_video_hours = 6  # from command line"), "{out}");
}

/// An empty `TOKULY_X=` line (env files are full of them) is not a value:
/// tokuly.toml applies, and the start says the line is ignored.
#[test]
fn empty_variables_do_not_hide_the_file() {
    let path = file("empty.toml", "[media]\nmax_video_hours = 5\n");
    let (ok, out, err) = run(tokuly(&["config", "show"]).env("TOKULY_CONFIG", &path).env("TOKULY_MEDIA_MAX_VIDEO_HOURS", ""));
    assert!(ok, "{err}");
    assert!(out.contains("max_video_hours = 5  # from tokuly.toml"), "{out}");
    assert!(err.contains("TOKULY_MEDIA_MAX_VIDEO_HOURS is empty and ignored"), "{err}");
}

/// Where empty means something (no bonus, no trusted proxies), an empty
/// variable is that value, as it always was.
#[test]
fn empty_stays_a_value_where_it_means_something() {
    let (ok, out, err) = run(tokuly(&["config", "show"]).env("TOKULY_WALLET_DAILY_BONUS_COINS", "").env("TOKULY_TRUSTED_PROXIES", ""));
    assert!(ok, "{err}");
    assert!(out.contains("daily_bonus_coins = \"\"  # from 環境変数"), "{out}");
    assert!(out.contains("trusted_proxies = []  # from 環境変数"), "{out}");
    assert!(!err.contains("is empty and ignored"), "{err}");
}

#[test]
fn typos_and_wrong_types_stop_the_start() {
    for (text, expected) in [
        ("[media]\nmax_video_hourz = 5\n", "unknown setting [media] max_video_hourz (did you mean `max_video_hours`?)"),
        ("[media]\nmax_video_hours = \"five\"\n", "[media] max_video_hours: expected an integer, found string"),
        ("[medai]\nmax_video_hours = 5\n", "unknown section [medai] (did you mean `media`?)"),
        ("[core]\nrate_limits = \"no\"\n", "[core] rate_limits: expected true or false"),
    ] {
        let path = file("bad.toml", text);
        // A server role refuses to start before it connects to anything.
        let (ok, _, err) = run(tokuly(&["api"]).env("TOKULY_CONFIG", &path).env("TOKULY_SECRET", "s".repeat(40)));
        assert!(!ok && err.contains(expected), "{text}: {err}");
    }
}

#[test]
fn a_missing_file_is_no_file() {
    let (ok, out, err) = run(tokuly(&["config", "show"]).env("TOKULY_CONFIG", "/nonexistent/tokuly.toml"));
    assert!(ok, "{err}");
    assert!(err.contains("does not exist"), "{err}");
    assert!(!out.contains("[media]"), "{out}");
}

#[test]
fn the_example_command_prints_every_section() {
    let (ok, out, err) = run(&mut tokuly(&["config", "example"]));
    assert!(ok, "{err}");
    for section in ["[core]", "[api]", "[realtime]", "[media]", "[chat]", "[engagement]"] {
        assert!(out.contains(&format!("\n{section}\n")), "{section}");
    }
}
