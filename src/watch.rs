//! `wait` と `watch`: board の変化を、注意が要るものだけ、まとめて伝える。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use clap::Args;
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use serde_json::json;

use crate::board::Board;
use crate::entry::Entry;
use crate::sid::Sid;
use crate::status::Status;

pub const POLL: Duration = Duration::from_secs(2);
pub const COALESCE: Duration = Duration::from_secs(1);

#[derive(Args, Debug)]
pub struct WaitArgs {
    /// この sid の直接の子を対象にする（既定: $LSBOARD_SID）
    #[arg(long, env = "LSBOARD_SID", value_name = "SID")]
    pub children_of: Option<String>,
    /// どれか 1 つが条件を満たしたら戻る（既定）
    #[arg(long, conflicts_with = "all")]
    pub any: bool,
    /// 全員が条件を満たすまで待つ
    #[arg(long)]
    pub all: bool,
    /// 待つ status（既定: waiting,idle,ended）
    #[arg(long, value_delimiter = ',', value_name = "STATUS")]
    pub until: Vec<Status>,
    /// 秒。過ぎたら終了コード 2 で戻る
    #[arg(long, value_name = "SEC")]
    pub timeout: Option<f64>,
    /// 1 行 1 JSON で出す
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// この sid の直接の子だけを流す（既定: $LSBOARD_SID。なければ全員）
    #[arg(long, env = "LSBOARD_SID", value_name = "SID")]
    pub children_of: Option<String>,
    /// 遷移を流す status（既定: waiting,idle,ended）
    #[arg(long, value_delimiter = ',', value_name = "STATUS")]
    pub until: Vec<Status>,
    /// 1 行 1 JSON で出す
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Appeared,
    Changed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: Kind,
    pub entry: Entry,
    pub prev: Option<Status>,
}

pub type Snapshot = HashMap<Sid, Entry>;

pub fn snapshot(board: &Board) -> std::io::Result<Snapshot> {
    // 生きている entry を先に読み、`.ended/` は穴を埋めるだけにする。
    // 逆順だと、読んでいる最中の rename が「90-ended で現れた」と見えることがある。
    let mut m = HashMap::new();
    for e in board.entries()? {
        m.insert(e.sid.clone(), e);
    }
    for e in board.ended()? {
        m.entry(e.sid.clone()).or_insert(e);
    }
    Ok(m)
}

/// 生きている entry だけのスナップショット（初期判定に使う）。
pub fn live_snapshot(board: &Board) -> std::io::Result<Snapshot> {
    Ok(board
        .entries()?
        .into_iter()
        .map(|e| (e.sid.clone(), e))
        .collect())
}

fn in_scope(e: &Entry, scope: Option<&Sid>) -> bool {
    scope.is_none_or(|p| e.sid.is_child_of(p))
}

/// 2 つのスナップショットの差から、伝える価値のある変化を取り出す。
pub fn diff(old: &Snapshot, new: &Snapshot, scope: Option<&Sid>, until: &[Status]) -> Vec<Event> {
    let mut out = Vec::new();
    for (sid, e) in new {
        if !in_scope(e, scope) {
            continue;
        }
        match old.get(sid) {
            None => out.push(Event {
                kind: Kind::Appeared,
                entry: e.clone(),
                prev: None,
            }),
            Some(prev) if prev.status != e.status && until.contains(&e.status) => out.push(Event {
                kind: Kind::Changed,
                entry: e.clone(),
                prev: Some(prev.status),
            }),
            Some(_) => {}
        }
    }
    out.sort_by(|a, b| a.entry.sid.cmp(&b.entry.sid));
    out
}

/// 同じ時間窓の変化を sid ごとに 1 つにまとめる。最新の状態を残し、
/// 窓の最初が「現れた」ならその種別を保つ。
pub fn merge(pending: &mut Vec<Event>, events: Vec<Event>) {
    for ev in events {
        match pending.iter_mut().find(|p| p.entry.sid == ev.entry.sid) {
            Some(p) => {
                p.entry = ev.entry;
                if p.kind != Kind::Appeared {
                    p.kind = ev.kind;
                }
            }
            None => pending.push(ev),
        }
    }
    pending.sort_by(|a, b| a.entry.sid.cmp(&b.entry.sid));
}

pub fn format_line(e: &Entry, kind: Option<Kind>, json: bool) -> String {
    if json {
        json!({
            "event": kind.map(|k| match k { Kind::Appeared => "appeared", Kind::Changed => "changed" }),
            "sid": e.sid.as_str(),
            "status": e.status.to_string(),
            "agent": e.agent,
            "name": e.name,
            "message": e.message,
        })
        .to_string()
    } else if e.message.is_empty() {
        format!("{} {} {}", e.sid, e.status, e.name)
    } else {
        format!("{} {} {} {}", e.sid, e.status, e.name, e.message)
    }
}

struct Monitor {
    rx: Receiver<()>,
    _watcher: Option<RecommendedWatcher>,
}

impl Monitor {
    fn start(board: &Board) -> Monitor {
        let (tx, rx) = mpsc::channel();
        let watcher = notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
            let _ = tx.send(());
        })
        .ok()
        .and_then(|mut w| {
            let dirs: [PathBuf; 2] = [board.dir.clone(), board.ended_dir()];
            for d in dirs {
                w.watch(&d, RecursiveMode::NonRecursive).ok()?;
            }
            Some(w)
        });
        if watcher.is_none() {
            eprintln!(
                "lsboard: file watching unavailable; polling every {}s",
                POLL.as_secs()
            );
        }
        Monitor {
            rx,
            _watcher: watcher,
        }
    }

    /// 変化の知らせか timeout まで待つ。知らせは全部読み捨てる。
    fn wait(&self, timeout: Duration) {
        match self.rx.recv_timeout(timeout) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(timeout),
        }
        while self.rx.try_recv().is_ok() {}
    }
}

fn parse_scope(s: Option<&str>) -> Result<Option<Sid>, String> {
    match s {
        None => Ok(None),
        Some(x) => Sid::parse(x)
            .map(Some)
            .ok_or_else(|| format!("invalid sid: {x}")),
    }
}

fn until_or_default(u: &[Status]) -> Vec<Status> {
    if u.is_empty() {
        Status::ATTENTION.to_vec()
    } else {
        u.to_vec()
    }
}

/// scope の中で、いま until に入っている entry（sid 順）。
fn matching(snap: &Snapshot, scope: Option<&Sid>, until: &[Status]) -> Vec<Entry> {
    let mut v: Vec<Entry> = snap
        .values()
        .filter(|e| in_scope(e, scope) && until.contains(&e.status))
        .cloned()
        .collect();
    v.sort_by(|a, b| a.sid.cmp(&b.sid));
    v
}

/// `--all`: 開始時に生きていた子（tracked）が全員 until に入ったか。
/// スナップショットから消えた子は、until に ended があれば終わったとみなす。
fn all_done(snap: &Snapshot, tracked: &[Sid], until: &[Status]) -> bool {
    tracked.iter().all(|sid| match snap.get(sid) {
        Some(e) => until.contains(&e.status),
        None => until.contains(&Status::Ended),
    })
}

fn deadline_from(timeout: Option<f64>) -> Option<Instant> {
    let secs = timeout?;
    let d = Duration::try_from_secs_f64(secs.max(0.0)).unwrap_or(Duration::MAX);
    Instant::now().checked_add(d)
}

struct Coalescer {
    pending: Vec<Event>,
    first: Option<Instant>,
}

impl Coalescer {
    fn new() -> Coalescer {
        Coalescer {
            pending: Vec::new(),
            first: None,
        }
    }

    fn push(&mut self, events: Vec<Event>) {
        if events.is_empty() {
            return;
        }
        merge(&mut self.pending, events);
        self.first.get_or_insert(Instant::now());
    }

    fn ready(&self) -> bool {
        self.first.is_some_and(|f| f.elapsed() >= COALESCE)
    }

    /// 次に起きるべき時刻までの長さ（なければ POLL）。
    fn sleep_for(&self, now: Instant) -> Duration {
        match self.first {
            Some(f) => POLL.min((f + COALESCE).saturating_duration_since(now)),
            None => POLL,
        }
    }

    fn take(&mut self) -> Vec<Event> {
        self.first = None;
        std::mem::take(&mut self.pending)
    }
}

/// `lsboard wait`。戻り値は終了コード（0: 満たした、2: timeout か対象なし）。
pub fn wait(board_arg: Option<PathBuf>, args: WaitArgs) -> Result<u8, String> {
    let board = Board::open(board_arg).map_err(|e| e.to_string())?;
    let scope = parse_scope(args.children_of.as_deref())?;
    let Some(parent) = scope.clone() else {
        return Err("wait needs --children-of SID (or LSBOARD_SID)".into());
    };
    let until = until_or_default(&args.until);
    let all = args.all;
    let deadline = deadline_from(args.timeout);
    let monitor = Monitor::start(&board);

    // 初期判定は生きている entry だけで行う。`.ended/` の古い entry で
    // すぐ戻ってしまわないため。
    let live = live_snapshot(&board).map_err(|e| e.to_string())?;
    let mut tracked: Vec<Sid> = live
        .values()
        .filter(|e| in_scope(e, Some(&parent)))
        .map(|e| e.sid.clone())
        .collect();
    tracked.sort();
    if all && tracked.is_empty() {
        eprintln!("lsboard wait: no live children of {parent}");
        return Ok(2);
    }
    let now_matching = matching(&live, Some(&parent), &until);
    let satisfied_now = if all {
        all_done(&live, &tracked, &until)
    } else {
        !now_matching.is_empty()
    };
    if satisfied_now {
        for e in &now_matching {
            println!("{}", format_line(e, None, args.json));
        }
        return Ok(0);
    }

    let mut prev = snapshot(&board).map_err(|e| e.to_string())?;
    let mut buf = Coalescer::new();
    loop {
        let now = Instant::now();
        let timed_out = deadline.is_some_and(|d| now >= d);
        if !timed_out {
            let mut sleep = buf.sleep_for(now);
            if let Some(d) = deadline {
                sleep = sleep.min(d.saturating_duration_since(now));
            }
            monitor.wait(sleep);
        }

        let snap = snapshot(&board).map_err(|e| e.to_string())?;
        buf.push(diff(&prev, &snap, Some(&parent), &until));
        prev = snap;

        if buf.ready() || (timed_out && !buf.pending.is_empty()) {
            let events = buf.take();
            for ev in &events {
                println!("{}", format_line(&ev.entry, Some(ev.kind), args.json));
            }
            // 条件は、溜めた変化ではなく最後のスナップショットで判定する。
            let done = if all {
                all_done(&prev, &tracked, &until)
            } else {
                events.iter().any(|ev| {
                    prev.get(&ev.entry.sid)
                        .is_some_and(|e| until.contains(&e.status))
                })
            };
            if done {
                return Ok(0);
            }
        }
        if timed_out {
            return Ok(2);
        }
    }
}

/// `lsboard watch`。止められるまで流し続ける。
pub fn watch(board_arg: Option<PathBuf>, args: WatchArgs) -> Result<u8, String> {
    use std::io::Write as _;
    let board = Board::open(board_arg).map_err(|e| e.to_string())?;
    let scope = parse_scope(args.children_of.as_deref())?;
    let until = until_or_default(&args.until);
    let monitor = Monitor::start(&board);
    let mut prev = snapshot(&board).map_err(|e| e.to_string())?;
    let mut buf = Coalescer::new();
    let stdout = std::io::stdout();
    loop {
        monitor.wait(buf.sleep_for(Instant::now()));
        let snap = snapshot(&board).map_err(|e| e.to_string())?;
        buf.push(diff(&prev, &snap, scope.as_ref(), &until));
        prev = snap;
        if buf.ready() {
            let mut out = stdout.lock();
            for ev in buf.take() {
                if writeln!(out, "{}", format_line(&ev.entry, Some(ev.kind), args.json)).is_err() {
                    return Ok(0);
                }
            }
            if out.flush().is_err() {
                return Ok(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(sid: &str, st: Status) -> Entry {
        Entry::new(st, Sid::parse(sid).unwrap(), "claude", "w")
    }

    fn snap(v: &[Entry]) -> Snapshot {
        v.iter().map(|e| (e.sid.clone(), e.clone())).collect()
    }

    #[test]
    fn diff_reports_appearance_and_attention_transitions_only() {
        let root = Sid::parse("k3f9").unwrap();
        let old = snap(&[
            e("k3f9.01", Status::Working),
            e("k3f9.02", Status::Idle),
            e("m7q2", Status::Working),
        ]);
        let new = snap(&[
            e("k3f9.01", Status::Background), // not in until → ignored
            e("k3f9.02", Status::Working),    // idle → working: ignored
            e("k3f9.03", Status::Working),    // appeared
            e("m7q2", Status::Idle),          // out of scope
            e("k3f9.01.01", Status::Idle),    // grandchild: out of scope
        ]);
        let ev = diff(&old, &new, Some(&root), &Status::ATTENTION);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, Kind::Appeared);
        assert_eq!(ev[0].entry.sid.as_str(), "k3f9.03");

        let new2 = snap(&[e("k3f9.01", Status::Waiting), e("k3f9.02", Status::Ended)]);
        let ev2 = diff(&old, &new2, Some(&root), &Status::ATTENTION);
        assert_eq!(ev2.len(), 2);
        assert_eq!(ev2[0].prev, Some(Status::Working));
        assert_eq!(ev2[1].entry.status, Status::Ended);

        let all = diff(&old, &new2, None, &Status::ATTENTION);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn matching_and_all_done() {
        let root = Sid::parse("k3f9").unwrap();
        let s = snap(&[e("k3f9.01", Status::Working), e("k3f9.02", Status::Idle)]);
        assert_eq!(matching(&s, Some(&root), &Status::ATTENTION).len(), 1);
        let tracked = [
            Sid::parse("k3f9.01").unwrap(),
            Sid::parse("k3f9.02").unwrap(),
        ];
        assert!(!all_done(&s, &tracked, &Status::ATTENTION));
        let s2 = snap(&[e("k3f9.02", Status::Idle)]); // .01 vanished entirely
        assert!(all_done(&s2, &tracked, &Status::ATTENTION));
        assert!(!all_done(&s2, &tracked, &[Status::Idle]));
        let s3 = snap(&[e("k3f9.01", Status::Ended), e("k3f9.02", Status::Idle)]);
        assert!(all_done(&s3, &tracked, &Status::ATTENTION));
    }

    #[test]
    fn merge_keeps_latest_state_per_sid_and_first_kind() {
        let mut pending = Vec::new();
        let ev = |sid: &str, st: Status, kind: Kind| Event {
            kind,
            entry: e(sid, st),
            prev: None,
        };
        merge(
            &mut pending,
            vec![ev("k3f9.02", Status::Working, Kind::Appeared)],
        );
        merge(
            &mut pending,
            vec![
                ev("k3f9.02", Status::Idle, Kind::Changed),
                ev("k3f9.01", Status::Waiting, Kind::Changed),
            ],
        );
        merge(
            &mut pending,
            vec![ev("k3f9.01", Status::Ended, Kind::Changed)],
        );
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].entry.sid.as_str(), "k3f9.01");
        assert_eq!(pending[0].entry.status, Status::Ended);
        assert_eq!(pending[0].kind, Kind::Changed);
        assert_eq!(pending[1].entry.status, Status::Idle);
        assert_eq!(pending[1].kind, Kind::Appeared);
    }

    #[test]
    fn deadline_handles_infinity_and_negative() {
        assert!(deadline_from(None).is_none());
        assert!(deadline_from(Some(f64::INFINITY)).is_none());
        assert!(deadline_from(Some(f64::NAN)).unwrap() <= Instant::now()); // NaN.max(0) == 0
        let d = deadline_from(Some(-5.0)).unwrap();
        assert!(d <= Instant::now());
        assert!(deadline_from(Some(1.5)).unwrap() > Instant::now());
    }

    #[test]
    fn snapshot_prefers_live_over_ended() {
        let d = tempfile::tempdir().unwrap();
        let b = Board::init(Some(d.path().join("b"))).unwrap();
        let sid = b.reserve_root().unwrap();
        b.create(&Entry::new(Status::Working, sid.clone(), "claude", "w"))
            .unwrap();
        // a stale copy in .ended/ next to the live entry
        std::fs::File::create(b.ended_dir().join(format!("90-ended__{sid}__claude__w"))).unwrap();
        let s = snapshot(&b).unwrap();
        assert_eq!(s[&sid].status, Status::Working);
        assert_eq!(live_snapshot(&b).unwrap().len(), 1);
    }

    #[test]
    fn line_format() {
        let mut x = e("k3f9.01", Status::Waiting);
        x.message = "approve Bash".into();
        assert_eq!(
            format_line(&x, None, false),
            "k3f9.01 10-waiting w approve Bash"
        );
        let j: serde_json::Value =
            serde_json::from_str(&format_line(&x, Some(Kind::Changed), true)).unwrap();
        assert_eq!(j["event"], "changed");
        assert_eq!(j["status"], "10-waiting");
    }
}
