//! `lsboard ls`: 整形して見る。生の `ls` でも読めるが、列をそろえ、家族を木にする。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::board::Board;
use crate::entry::Entry;

fn width(s: &str) -> usize {
    s.chars().count()
}

fn pad(s: &str, w: usize) -> String {
    let mut out = s.to_string();
    for _ in width(s)..w {
        out.push(' ');
    }
    out
}

fn updated_secs(board: &Board, e: &Entry) -> Option<u64> {
    let path = board.dir.join(e.filename());
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

fn started_secs(board: &Board, e: &Entry) -> Option<u64> {
    board
        .started_at(&e.sid)
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

/// 行の並び: `--tree` なら sid 順（親が子の前に来る）、それ以外は status 順。
fn rows(board: &Board, tree: bool) -> std::io::Result<Vec<Entry>> {
    let mut v = board.entries()?;
    if tree {
        v.sort_by(|a, b| a.sid.cmp(&b.sid));
    }
    Ok(v)
}

pub fn ls(board_arg: Option<PathBuf>, tree: bool, json: bool) -> Result<u8, String> {
    let board = Board::open(board_arg).map_err(|e| e.to_string())?;
    let entries = rows(&board, tree).map_err(|e| e.to_string())?;
    let live_children = |e: &Entry| entries.iter().filter(|c| c.sid.is_child_of(&e.sid)).count();

    if json {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let items: Vec<_> = entries
            .iter()
            .map(|e| {
                json!({
                    "sid": e.sid.as_str(),
                    "status": e.status.to_string(),
                    "agent": e.agent,
                    "name": e.name,
                    "message": e.message,
                    "children": live_children(e),
                    "started_at": started_secs(&board, e),
                    "updated_at": updated_secs(&board, e),
                    "age_secs": updated_secs(&board, e).map(|u| now.saturating_sub(u)),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&items).map_err(|e| e.to_string())?
        );
        return Ok(0);
    }

    let mut lines: Vec<[String; 5]> = Vec::new();
    for e in &entries {
        let sid = if tree {
            format!("{}{}", "  ".repeat(e.sid.depth()), e.sid)
        } else {
            e.sid.to_string()
        };
        let mut tail = e.message.clone();
        let n = live_children(e);
        if tree && n > 0 {
            if !tail.is_empty() {
                tail.push(' ');
            }
            tail.push_str(&format!(
                "[{n} live child{}]",
                if n == 1 { "" } else { "ren" }
            ));
        }
        lines.push([
            e.status.to_string(),
            sid,
            e.agent.clone(),
            e.name.clone(),
            tail,
        ]);
    }
    let mut w = [0usize; 4];
    for l in &lines {
        for (i, col) in l.iter().take(4).enumerate() {
            w[i] = w[i].max(width(col));
        }
    }
    for l in &lines {
        let mut s = String::new();
        for (i, col) in l.iter().take(4).enumerate() {
            s.push_str(&pad(col, w[i]));
            s.push_str("  ");
        }
        s.push_str(&l[4]);
        println!("{}", s.trim_end());
    }
    Ok(0)
}
