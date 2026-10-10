//! Claude Code の hook の遷移表（concept §6）。
//!
//! 入力は標準入力の JSON。`hook_event_name` で分岐する。

use std::fmt::Write as _;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::Ctx;
use crate::entry::{self, Entry};
use crate::lock::HOOK_WAIT;
use crate::sid::Sid;
use crate::status::Status;

pub const EVENTS: [&str; 9] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "SessionEnd",
];

/// `matcher: "*"` を付けて登録するイベント。
pub fn needs_matcher(event: &str) -> bool {
    matches!(event, "PreToolUse" | "PermissionRequest" | "PostToolUse")
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|x| !x.is_empty())
}

/// ツール名と入力から、承認の要求とツール実行後で同じになる識別子を作る。
/// AskUserQuestion は回答後に `answers` と `annotations` が入力に加わるので除く。
///
/// 鍵の順序の正規化は serde_json の既定（`Map` が BTreeMap）に頼っている。
/// `preserve_order` feature を有効にすると入力の順序に依存して一致しなくなるので、有効にしない。
pub fn tool_hash(tool: &str, input: &Value) -> String {
    let mut input = input.clone();
    if let Some(o) = input.as_object_mut() {
        o.remove("answers");
        o.remove("annotations");
    }
    let canonical = if input.is_null() {
        "{}".to_string()
    } else {
        serde_json::to_string(&input).unwrap_or_default()
    };
    let mut h = Sha256::new();
    h.update(tool.as_bytes());
    h.update(b"\n");
    h.update(canonical.as_bytes());
    let mut out = String::new();
    for b in h.finalize() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// hook の対象の entry。サブエージェントの中のイベントなら、その子の entry。
fn target(ctx: &Ctx, v: &Value) -> Sid {
    if let Some(id) = s(v, "agent_id")
        && let Ok(Some(child)) = ctx.board.find_by_meta("agent_id", id)
    {
        return child;
    }
    ctx.sid.clone()
}

fn waiting_by_hook(ctx: &Ctx, sid: &Sid) -> Option<String> {
    ctx.board
        .meta_read(sid, "waiting")
        .and_then(|w| w.strip_prefix("hook ").map(str::to_string))
}

fn waiting_by_agent(ctx: &Ctx, sid: &Sid) -> bool {
    ctx.board.meta_read(sid, "waiting").as_deref() == Some("agent")
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn context_text(ctx: &Ctx) -> String {
    format!(
        "lsboard: you are sid {} on board {}.\n\
         - Declare a human-attention wait:  lsboard set waiting \"what you need\"\n\
         - Back to work / idle:             lsboard set working | lsboard set idle\n\
         - Wait for your children cheaply:  lsboard wait   (blocks; run it in the background)\n\
         - See everyone:                    lsboard ls --tree\n",
        ctx.sid,
        ctx.board.dir.display()
    )
}

pub fn handle(ctx: &Ctx, v: &Value) -> Result<(), String> {
    let b = &ctx.board;
    let event = s(v, "hook_event_name").unwrap_or("");
    match event {
        "SessionStart" => {
            if let Some(id) = s(v, "session_id") {
                b.meta_write(&ctx.sid, "session", id).map_err(err)?;
            }
            if let Some(t) = s(v, "transcript_path") {
                b.meta_write(&ctx.sid, "transcript", t).map_err(err)?;
            }
            print!("{}", context_text(ctx));
        }
        "UserPromptSubmit" => {
            let t = target(ctx, v);
            let auto = s(v, "prompt")
                .map(|p| p.trim_start().starts_with("<task-notification>"))
                .unwrap_or(false);
            if auto {
                b.update(&t, HOOK_WAIT, |e| {
                    set_if(e, &[Status::Idle, Status::Background], Status::Working)
                })
                .map_err(err)?;
            } else {
                b.update(&t, HOOK_WAIT, |e| {
                    e.status = Status::Working;
                    e.message.clear();
                    true
                })
                .map_err(err)?;
                b.meta_remove(&t, "waiting");
            }
        }
        "PreToolUse" => {
            let t = target(ctx, v);
            b.update(&t, HOOK_WAIT, |e| {
                set_if(e, &[Status::Idle, Status::Background], Status::Working)
            })
            .map_err(err)?;
        }
        "PermissionRequest" => {
            let t = target(ctx, v);
            if waiting_by_agent(ctx, &t) {
                b.touch(&t).map_err(err)?;
                return Ok(());
            }
            let tool = s(v, "tool_name").unwrap_or("tool");
            let hash = tool_hash(tool, v.get("tool_input").unwrap_or(&Value::Null));
            let message = if tool == "AskUserQuestion" {
                "question".to_string()
            } else {
                entry::sanitize_message(&format!("approve {tool}"))
            };
            let updated = b
                .update(&t, HOOK_WAIT, |e| {
                    e.status = Status::Waiting;
                    e.message = message;
                    true
                })
                .map_err(err)?;
            if updated.is_some() {
                b.meta_write(&t, "waiting", &format!("hook {hash}"))
                    .map_err(err)?;
            }
        }
        "PostToolUse" => {
            let t = target(ctx, v);
            let tool = s(v, "tool_name").unwrap_or("tool");
            let hash = tool_hash(tool, v.get("tool_input").unwrap_or(&Value::Null));
            if waiting_by_hook(ctx, &t).as_deref() == Some(hash.as_str()) {
                b.update(&t, HOOK_WAIT, |e| {
                    e.status = Status::Working;
                    e.message.clear();
                    true
                })
                .map_err(err)?;
                b.meta_remove(&t, "waiting");
            } else {
                b.touch(&t).map_err(err)?;
            }
        }
        "Stop" => {
            let t = target(ctx, v);
            let running = v
                .get("background_tasks")
                .and_then(Value::as_array)
                .map(|a| a.iter().any(|x| s(x, "status") == Some("running")))
                .unwrap_or(false);
            if running {
                b.update(&t, HOOK_WAIT, |e| {
                    set_if(e, &[Status::Working], Status::Background)
                })
                .map_err(err)?;
            } else {
                let by_hook = waiting_by_hook(ctx, &t).is_some();
                let mut cleared = false;
                b.update(&t, HOOK_WAIT, |e| match e.status {
                    Status::Working | Status::Background => {
                        e.status = Status::Idle;
                        true
                    }
                    Status::Waiting if by_hook => {
                        e.status = Status::Idle;
                        e.message.clear();
                        cleared = true;
                        true
                    }
                    _ => false,
                })
                .map_err(err)?;
                if cleared {
                    b.meta_remove(&t, "waiting");
                }
            }
        }
        "SubagentStart" => {
            let Some(agent_id) = s(v, "agent_id") else {
                return Ok(());
            };
            if b.find_by_meta("agent_id", agent_id).map_err(err)?.is_some() {
                return Ok(());
            }
            let agent = b
                .meta_read(&ctx.sid, "agent")
                .unwrap_or_else(|| "claude".to_string());
            let name = s(v, "agent_type")
                .and_then(entry::to_token)
                .map(|t| entry::clip_name(&t))
                .unwrap_or_else(|| "subagent".to_string());
            let child = b.reserve_child(&ctx.sid).map_err(err)?;
            // entry を作れなければ予約ごと消す。残すと agent_id の meta が
            // 以後のイベントを吸い込んでしまう。
            let created = (|| -> std::io::Result<()> {
                b.meta_write(&child, "agent", &agent)?;
                b.meta_write(&child, "agent_id", agent_id)?;
                b.meta_write(&child, "name", &name)?;
                if let Some(pid) = b.meta_read(&ctx.sid, "pid") {
                    b.meta_write(&child, "pid", &pid)?;
                }
                b.create(&Entry::new(Status::Working, child.clone(), &agent, &name))?;
                Ok(())
            })();
            if let Err(e) = created {
                let _ = b.release(&child);
                return Err(format!("subagent entry for {child}: {e}"));
            }
        }
        "SubagentStop" => {
            if let Some(agent_id) = s(v, "agent_id")
                && let Some(child) = b.find_by_meta("agent_id", agent_id).map_err(err)?
            {
                b.end(&child, HOOK_WAIT).map_err(err)?;
            }
        }
        // SessionEnd の reason は clear / logout / prompt_input_exit /
        // bypass_permissions_disabled / other。プロセスが本当に終わるのは
        // prompt_input_exit と logout だけなので、自分を終わらせるのはそのときだけ。
        // それ以外（/clear を含む）は touch にとどめる。本当の終了は `lsboard run` の
        // 後始末が拾う。サブエージェントの子はどの reason でも終わらせる。
        "SessionEnd" => {
            end_subagent_children(ctx)?;
            match s(v, "reason") {
                Some("prompt_input_exit" | "logout") => {
                    b.end(&ctx.sid, HOOK_WAIT).map_err(err)?;
                }
                _ => {
                    b.touch(&ctx.sid).map_err(err)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// このセッションの hook が作った子（`agent_id` を持つ子孫）を終わらせる。
/// `lsboard run --parent` で起動した別プロセスの子は対象にしない。
fn end_subagent_children(ctx: &Ctx) -> Result<(), String> {
    let b = &ctx.board;
    let mut subs: Vec<Sid> = b
        .entries()
        .map_err(err)?
        .into_iter()
        .map(|e| e.sid)
        .filter(|sid| sid.is_descendant_of(&ctx.sid) && b.meta_read(sid, "agent_id").is_some())
        .collect();
    subs.sort_by_key(|s| std::cmp::Reverse(s.depth()));
    for sid in subs {
        if let Err(e) = b.end(&sid, HOOK_WAIT) {
            eprintln!("lsboard hook: could not end subagent {sid}: {e}");
        }
    }
    Ok(())
}

fn set_if(e: &mut Entry, from: &[Status], to: Status) -> bool {
    if from.contains(&e.status) {
        e.status = to;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_hash_ignores_answers_and_key_order() {
        let a = tool_hash("AskUserQuestion", &json!({"questions": [1], "b": 2}));
        let b = tool_hash(
            "AskUserQuestion",
            &json!({"b": 2, "questions": [1], "answers": {"x": "y"}, "annotations": {}}),
        );
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, tool_hash("Bash", &json!({"questions": [1], "b": 2})));
        assert_eq!(tool_hash("X", &Value::Null), tool_hash("X", &json!({})));
    }
}
