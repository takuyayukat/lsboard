//! `lsboard run`: sid を予約し、entry を作り、hook を登録してエージェントを起動する。

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use clap::Args;
use serde_json::{Value, json};

use crate::board::Board;
use crate::entry::{self, Entry};
use crate::hooks::claude;
use crate::lock::DEFAULT_WAIT;
use crate::sid::Sid;
use crate::status::Status;
use crate::sys;

#[derive(Args, Debug)]
pub struct RunArgs {
    /// entry の呼び名（省略時はコマンド名）
    #[arg(long)]
    pub name: Option<String>,
    /// 親の sid。あればその子として予約する
    #[arg(long)]
    pub parent: Option<String>,
    /// hook を登録しない（起動と終了だけ board に載せる）
    #[arg(long)]
    pub no_hooks: bool,
    /// エージェントの種類を上書きする（既定はコマンド名）。ラッパー経由でも `claude` の hook を使える
    #[arg(long, value_name = "AGENT")]
    pub agent: Option<String>,
    /// 起動するコマンドと引数
    #[arg(
        required = true,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "COMMAND"
    )]
    pub command: Vec<String>,
}

fn agent_of(cmd: &str) -> String {
    let base = std::path::Path::new(cmd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| cmd.to_string());
    entry::to_token(&base).unwrap_or_else(|| "agent".to_string())
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Claude Code 用の `--settings` JSON。
pub fn claude_settings(exe: &str) -> Value {
    let command = format!("{} hook claude", shell_quote(exe));
    let mut hooks = serde_json::Map::new();
    for ev in claude::EVENTS {
        let mut group = serde_json::Map::new();
        if claude::needs_matcher(ev) {
            group.insert("matcher".into(), json!("*"));
        }
        group.insert(
            "hooks".into(),
            json!([{"type": "command", "command": command}]),
        );
        hooks.insert(ev.to_string(), json!([Value::Object(group)]));
    }
    json!({ "hooks": hooks })
}

pub fn run(board_arg: Option<PathBuf>, args: RunArgs) -> Result<u8, String> {
    let board = Board::open(board_arg).map_err(|e| e.to_string())?;
    let agent = match &args.agent {
        Some(a) => entry::to_token(a).ok_or_else(|| format!("invalid agent: {a}"))?,
        None => agent_of(&args.command[0]),
    };
    if !args.no_hooks && agent != "claude" {
        eprintln!("lsboard run: no hook adapter for {agent}; only start/end will be recorded");
    }
    let name = match &args.name {
        Some(n) => {
            let t = entry::to_token(n).ok_or_else(|| format!("invalid name: {n}"))?;
            if t.len() > entry::MAX_NAME_CHARS {
                return Err(format!(
                    "name is too long (max {} chars): {t}",
                    entry::MAX_NAME_CHARS
                ));
            }
            t
        }
        // --agent で上書きしても、呼び名はコマンド名のままにする
        None => agent_of(&args.command[0]),
    };

    let sid = match &args.parent {
        Some(p) => {
            let parent = Sid::parse(p).ok_or_else(|| format!("invalid parent sid: {p}"))?;
            board.reserve_child(&parent).map_err(|e| e.to_string())?
        }
        None => board.reserve_root().map_err(|e| e.to_string())?,
    };
    board
        .meta_write(&sid, "agent", &agent)
        .map_err(|e| e.to_string())?;
    board
        .meta_write(&sid, "name", &name)
        .map_err(|e| e.to_string())?;
    board
        .meta_write(&sid, "pid", &std::process::id().to_string())
        .map_err(|e| e.to_string())?;
    board
        .create(&Entry::new(Status::Idle, sid.clone(), &agent, &name))
        .map_err(|e| e.to_string())?;

    let mut argv = args.command.clone();
    let mut settings_path: Option<PathBuf> = None;
    if !args.no_hooks && agent == "claude" {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let path = board.sid_dir(&sid).join("settings.json");
        let json = serde_json::to_string(&claude_settings(&exe.to_string_lossy()))
            .map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())?;
        argv.insert(1, "--settings".to_string());
        argv.insert(2, path.to_string_lossy().into_owned());
        settings_path = Some(path);
    }

    sys::install_parent_handlers();
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .env("LSBOARD_SID", sid.as_str())
        .env("LSBOARD_DIR", &board.dir);
    sys::reset_in_child(&mut cmd);

    let code = match cmd.spawn() {
        Ok(mut child) => {
            let mut forwarded = false;
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => break sys::exit_code_of(&status),
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("lsboard run: wait: {e}");
                        break 1;
                    }
                }
                if !forwarded && sys::received_signal() != 0 {
                    sys::terminate(child.id());
                    forwarded = true;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
        Err(e) => {
            eprintln!("lsboard run: cannot start {}: {e}", argv[0]);
            127
        }
    };

    if let Err(e) = board.end_tree(&sid, DEFAULT_WAIT) {
        let left = board
            .find(&sid)
            .ok()
            .flatten()
            .map(|(p, _)| p.display().to_string())
            .unwrap_or_else(|| sid.to_string());
        eprintln!("lsboard run: entry was left on the board, end it by hand: {left} ({e})");
    }
    if let Some(p) = settings_path {
        let _ = fs::remove_file(p);
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_register_every_event() {
        let v = claude_settings("/opt/ls board/lsboard");
        let hooks = v["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), claude::EVENTS.len());
        let pre = &hooks["PreToolUse"][0];
        assert_eq!(pre["matcher"], "*");
        assert_eq!(
            pre["hooks"][0]["command"],
            "'/opt/ls board/lsboard' hook claude"
        );
        assert!(hooks["Stop"][0].get("matcher").is_none());
    }

    #[test]
    fn agent_token_from_command() {
        assert_eq!(agent_of("/usr/local/bin/claude"), "claude");
        assert_eq!(agent_of("sh"), "sh");
        assert_eq!(agent_of("My Agent.exe"), "my-agent-exe");
    }
}
