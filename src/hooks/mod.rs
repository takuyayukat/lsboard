//! hook の入口。エージェントごとの遷移表は子モジュールに置く。
//!
//! 共通の規則: `LSBOARD_SID` と `LSBOARD_DIR` がなければ何もしない。
//! 失敗してもエージェントを止めないため、常に終了コード 0 で終わる。

pub mod claude;

use std::io::Read;
use std::path::PathBuf;

use crate::board::Board;
use crate::sid::Sid;

pub struct Ctx {
    pub board: Board,
    pub sid: Sid,
}

fn run_inner(agent: &str) -> Result<(), String> {
    let Some(sid) = std::env::var_os("LSBOARD_SID") else {
        return Ok(());
    };
    let Some(dir) = std::env::var_os("LSBOARD_DIR") else {
        return Ok(());
    };
    let sid = Sid::parse(&sid.to_string_lossy()).ok_or("LSBOARD_SID is not a valid sid")?;
    let board = Board::open(Some(PathBuf::from(dir))).map_err(|e| e.to_string())?;
    let ctx = Ctx { board, sid };

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("read stdin: {e}"))?;
    if input.trim().is_empty() {
        return Ok(());
    }
    let value: serde_json::Value =
        serde_json::from_str(&input).map_err(|e| format!("parse hook input: {e}"))?;

    match agent {
        "claude" => claude::handle(&ctx, &value),
        other => Err(format!("unknown agent for hook: {other}")),
    }
}

/// `lsboard hook <agent>`。終了コードを返す（常に 0）。
pub fn run(agent: &str) -> u8 {
    let outcome = std::panic::catch_unwind(|| run_inner(agent));
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("lsboard hook: {e}"),
        Err(_) => eprintln!("lsboard hook: internal error"),
    }
    0
}
