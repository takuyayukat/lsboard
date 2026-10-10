//! `lsboard lint`: ファイルシステムは何でも受け付けるので、不整合をここで報告する。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::board::Board;
use crate::sid::Sid;
use crate::sys;

/// ロックや entry のない予約をこれより古ければ報告する。
pub const STALE: Duration = Duration::from_secs(60);

fn age(path: &std::path::Path) -> Option<Duration> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
}

pub fn findings(board: &Board) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    let live = board.entries()?;
    let ended = board.ended()?;

    for name in board.unparsable()? {
        out.push(format!("unparsable entry name: {name:?}"));
    }

    let mut by_sid: HashMap<&Sid, usize> = HashMap::new();
    for e in &live {
        *by_sid.entry(&e.sid).or_default() += 1;
    }
    let mut dups: Vec<_> = by_sid.iter().filter(|(_, n)| **n > 1).collect();
    dups.sort();
    for (sid, n) in dups {
        out.push(format!("duplicate entries for sid {sid}: {n}"));
    }

    let live_sids: Vec<&Sid> = live.iter().map(|e| &e.sid).collect();
    for e in &live {
        if !board.has_sid(&e.sid) {
            out.push(format!("entry without reservation: {}", e.sid));
        }
        if let Some(p) = e.sid.parent()
            && !live_sids.contains(&&p)
        {
            out.push(format!("child {} is live but its parent {p} is not", e.sid));
        }
        if let Some(pid) = board
            .meta_read(&e.sid, "pid")
            .and_then(|p| p.parse::<u32>().ok())
            && !sys::process_exists(pid)
        {
            out.push(format!(
                "process gone: {} (pid {pid}) is still on the board",
                e.sid
            ));
        }
    }

    let has_entry = |sid: &Sid| live.iter().chain(ended.iter()).any(|e| &e.sid == sid);
    for sid in board.reserved_sids()? {
        let dir = board.sid_dir(&sid);
        let lock = dir.join("lock");
        if let Some(a) = age(&lock)
            && a > STALE
        {
            out.push(format!(
                "stale lock: {} ({}s old)",
                lock.display(),
                a.as_secs()
            ));
        }
        if !has_entry(&sid)
            && let Some(a) = age(&dir)
            && a > STALE
        {
            out.push(format!(
                "reserved sid without entry: {sid} ({}s old)",
                a.as_secs()
            ));
        }
    }
    Ok(out)
}

pub fn lint(board_arg: Option<PathBuf>) -> Result<u8, String> {
    let board = Board::open(board_arg).map_err(|e| e.to_string())?;
    let f = findings(&board).map_err(|e| e.to_string())?;
    for line in &f {
        println!("{line}");
    }
    Ok(if f.is_empty() { 0 } else { 1 })
}
