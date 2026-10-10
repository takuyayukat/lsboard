//! board: entry を置くディレクトリと、その上の原子的な操作。

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::entry::Entry;
use crate::lock;
use crate::sid::{self, Sid};
use crate::status::Status;

pub const MARKER: &str = ".lsboard";
pub const SPEC: &str = "1";
pub const SIDS: &str = ".sids";
pub const ENDED: &str = ".ended";
pub const STARTED: &str = "started";

#[derive(Clone, Debug)]
pub struct Board {
    pub dir: PathBuf,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

impl Board {
    /// `~/.local/state/lsboard/board`
    pub fn default_dir() -> PathBuf {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/state/lsboard/board")
    }

    pub fn resolve(arg: Option<PathBuf>) -> PathBuf {
        arg.unwrap_or_else(Board::default_dir)
    }

    /// 既存の board を開く。`.lsboard` がなければエラー。
    pub fn open(arg: Option<PathBuf>) -> io::Result<Board> {
        let dir = Board::resolve(arg);
        if !dir.join(MARKER).is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "board not found at {} (run `lsboard init` first)",
                    dir.display()
                ),
            ));
        }
        Ok(Board {
            dir: fs::canonicalize(&dir)?,
        })
    }

    /// board を作る。すでにあればそのまま開く。
    pub fn init(arg: Option<PathBuf>) -> io::Result<Board> {
        let dir = Board::resolve(arg);
        fs::create_dir_all(dir.join(SIDS))?;
        fs::create_dir_all(dir.join(ENDED))?;
        let marker = dir.join(MARKER);
        if !marker.exists() {
            fs::write(&marker, format!("{SPEC}\n"))?;
        }
        Board::open(Some(dir))
    }

    pub fn sids_dir(&self) -> PathBuf {
        self.dir.join(SIDS)
    }

    pub fn ended_dir(&self) -> PathBuf {
        self.dir.join(ENDED)
    }

    pub fn sid_dir(&self, sid: &Sid) -> PathBuf {
        self.sids_dir().join(sid.as_str())
    }

    pub fn has_sid(&self, sid: &Sid) -> bool {
        self.sid_dir(sid).is_dir()
    }

    pub fn reserve_root(&self) -> io::Result<Sid> {
        let sid = sid::reserve_root(&self.sids_dir())?;
        self.mark_started(&sid)?;
        Ok(sid)
    }

    pub fn reserve_child(&self, parent: &Sid) -> io::Result<Sid> {
        let sid = sid::reserve_child(&self.sids_dir(), parent)?;
        self.mark_started(&sid)?;
        Ok(sid)
    }

    /// 予約の直後に 1 回だけ書く。この mtime が `started_at`。
    /// sid ディレクトリ自身の mtime はロックの mkdir や meta の書き込みで動くので使えない。
    fn mark_started(&self, sid: &Sid) -> io::Result<()> {
        self.meta_write(sid, STARTED, "")
    }

    /// `started_at`: sid を予約した時刻。
    pub fn started_at(&self, sid: &Sid) -> Option<SystemTime> {
        fs::metadata(self.sid_dir(sid).join(STARTED))
            .and_then(|m| m.modified())
            .ok()
    }

    /// 予約を取り消す（entry を作れなかったときなど）。
    pub fn release(&self, sid: &Sid) -> io::Result<()> {
        fs::remove_dir_all(self.sid_dir(sid))
    }

    // --- .sids/<sid>/<key> の小さなファイル ---

    pub fn meta_write(&self, sid: &Sid, key: &str, value: &str) -> io::Result<()> {
        fs::write(self.sid_dir(sid).join(key), value)
    }

    pub fn meta_read(&self, sid: &Sid, key: &str) -> Option<String> {
        fs::read_to_string(self.sid_dir(sid).join(key))
            .ok()
            .map(|s| s.trim_end().to_string())
    }

    pub fn meta_remove(&self, sid: &Sid, key: &str) {
        let _ = fs::remove_file(self.sid_dir(sid).join(key));
    }

    /// `.sids/` の中から、`<key>` の中身が `value` の sid を探す。
    pub fn find_by_meta(&self, key: &str, value: &str) -> io::Result<Option<Sid>> {
        for ent in fs::read_dir(self.sids_dir())? {
            let ent = ent?;
            let Some(name) = ent.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Some(sid) = Sid::parse(&name) else {
                continue;
            };
            if self.meta_read(&sid, key).as_deref() == Some(value) {
                return Ok(Some(sid));
            }
        }
        Ok(None)
    }

    /// 予約済みの sid を全部。
    pub fn reserved_sids(&self) -> io::Result<Vec<Sid>> {
        let mut v = Vec::new();
        for ent in fs::read_dir(self.sids_dir())? {
            let ent = ent?;
            if let Some(sid) = ent.file_name().to_str().and_then(Sid::parse) {
                v.push(sid);
            }
        }
        v.sort();
        Ok(v)
    }

    // --- entry ---

    fn read_entries(dir: &Path) -> io::Result<Vec<(PathBuf, Entry)>> {
        let mut v = Vec::new();
        for ent in fs::read_dir(dir)? {
            let ent = ent?;
            let name = ent.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') {
                continue;
            }
            if let Some(e) = Entry::parse(name) {
                v.push((ent.path(), e));
            }
        }
        v.sort_by(|a, b| (a.1.status, &a.1.sid).cmp(&(b.1.status, &b.1.sid)));
        Ok(v)
    }

    /// 生きている entry（status と sid の順）。
    pub fn entries(&self) -> io::Result<Vec<Entry>> {
        Ok(Self::read_entries(&self.dir)?
            .into_iter()
            .map(|(_, e)| e)
            .collect())
    }

    /// `.ended/` の entry。
    pub fn ended(&self) -> io::Result<Vec<Entry>> {
        Ok(Self::read_entries(&self.ended_dir())?
            .into_iter()
            .map(|(_, e)| e)
            .collect())
    }

    /// 解析できない（規約に合わない）ファイル名。lint 用。
    pub fn unparsable(&self) -> io::Result<Vec<String>> {
        let mut v = Vec::new();
        for ent in fs::read_dir(&self.dir)? {
            let ent = ent?;
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || Entry::parse(&name).is_some() {
                continue;
            }
            v.push(name.into_owned());
        }
        v.sort();
        Ok(v)
    }

    /// 自分の entry。複数あれば mtime が新しいものを使う（他は消さない）。
    pub fn find(&self, sid: &Sid) -> io::Result<Option<(PathBuf, Entry)>> {
        let mut best: Option<(SystemTime, PathBuf, Entry)> = None;
        for (p, e) in Self::read_entries(&self.dir)? {
            if &e.sid != sid {
                continue;
            }
            let m = fs::metadata(&p)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if best.as_ref().is_none_or(|(bm, _, _)| m > *bm) {
                best = Some((m, p, e));
            }
        }
        Ok(best.map(|(_, p, e)| (p, e)))
    }

    pub fn create(&self, entry: &Entry) -> io::Result<PathBuf> {
        if !self.has_sid(&entry.sid) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("sid is not reserved: {}", entry.sid),
            ));
        }
        let path = self.dir.join(entry.filename());
        File::create_new(&path)?;
        Ok(path)
    }

    /// ロックの下で自分の entry を書き換える。閉包が true を返せば rename する。
    /// entry がなければ何もせず None。
    pub fn update<F>(&self, sid: &Sid, lock_wait: Duration, f: F) -> io::Result<Option<Entry>>
    where
        F: FnOnce(&mut Entry) -> bool,
    {
        let _lock = lock::acquire(&self.sid_dir(sid), lock_wait)?;
        let Some((mut path, mut entry)) = self.find(sid)? else {
            return Ok(None);
        };
        let before = entry.clone();
        if f(&mut entry) && entry != before {
            let new_path = self.dir.join(entry.filename());
            fs::rename(&path, &new_path)?;
            path = new_path;
        }
        touch_file(&path)?;
        Ok(Some(entry))
    }

    /// 生存の印。名前は変えない。ロックも取らない。
    pub fn touch(&self, sid: &Sid) -> io::Result<bool> {
        match self.find(sid)? {
            Some((p, _)) => {
                touch_file(&p)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// `90-ended` にして `.ended/` に移す。
    pub fn end(&self, sid: &Sid, lock_wait: Duration) -> io::Result<bool> {
        let _lock = lock::acquire(&self.sid_dir(sid), lock_wait)?;
        let Some((path, mut entry)) = self.find(sid)? else {
            return Ok(false);
        };
        entry.status = Status::Ended;
        entry.message.clear();
        let dest = self.ended_dir().join(entry.filename());
        fs::rename(&path, &dest)?;
        touch_file(&dest)?;
        self.meta_remove(sid, "waiting");
        Ok(true)
    }

    /// 子孫（深いものから）と自分を終わらせる。終わらせた数を返す。
    /// 子孫の失敗は stderr に出して続け、自分の失敗はエラーとして返す。
    pub fn end_tree(&self, sid: &Sid, lock_wait: Duration) -> io::Result<usize> {
        let mut targets: Vec<Sid> = self
            .entries()?
            .into_iter()
            .map(|e| e.sid)
            .filter(|s| s.is_descendant_of(sid))
            .collect();
        targets.sort_by_key(|s| std::cmp::Reverse(s.depth()));
        let mut n = 0;
        for t in targets {
            match self.end(&t, lock_wait) {
                Ok(true) => n += 1,
                Ok(false) => {}
                Err(e) => eprintln!("lsboard: could not end {t}: {e}"),
            }
        }
        if self.end(sid, lock_wait)? {
            n += 1;
        }
        Ok(n)
    }
}

/// mtime を今にする。ファイルが消えていれば（rename の直後など）成功扱い。
pub fn touch_file(path: &Path) -> io::Result<()> {
    match File::options().write(true).open(path) {
        Ok(f) => f.set_modified(SystemTime::now()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> (tempfile::TempDir, Board) {
        let d = tempfile::tempdir().unwrap();
        let b = Board::init(Some(d.path().join("board"))).unwrap();
        (d, b)
    }

    #[test]
    fn init_is_idempotent_and_open_requires_marker() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("b");
        assert!(Board::open(Some(p.clone())).is_err());
        Board::init(Some(p.clone())).unwrap();
        Board::init(Some(p.clone())).unwrap();
        let b = Board::open(Some(p.clone())).unwrap();
        assert!(b.sids_dir().is_dir());
        assert!(b.ended_dir().is_dir());
        assert_eq!(fs::read_to_string(p.join(MARKER)).unwrap().trim(), SPEC);
    }

    #[test]
    fn create_update_end_lifecycle() {
        let (_d, b) = board();
        let sid = b.reserve_root().unwrap();
        let e = Entry::new(Status::Idle, sid.clone(), "claude", "worker");
        b.create(&e).unwrap();
        assert_eq!(b.entries().unwrap(), vec![e.clone()]);

        let updated = b
            .update(&sid, Duration::ZERO, |e| {
                e.status = Status::Waiting;
                e.message = "hello".into();
                true
            })
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, Status::Waiting);
        let (p, found) = b.find(&sid).unwrap().unwrap();
        assert_eq!(found, updated);
        assert!(p.ends_with("10-waiting__".to_string() + sid.as_str() + "__claude__worker hello"));

        assert!(b.end(&sid, Duration::ZERO).unwrap());
        assert!(b.entries().unwrap().is_empty());
        let ended = b.ended().unwrap();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].status, Status::Ended);
        assert_eq!(ended[0].message, "");
        assert!(!b.end(&sid, Duration::ZERO).unwrap());
        assert!(b.update(&sid, Duration::ZERO, |_| true).unwrap().is_none());
    }

    #[test]
    fn create_requires_reserved_sid() {
        let (_d, b) = board();
        let e = Entry::new(Status::Idle, Sid::parse("zzzz").unwrap(), "claude", "x");
        assert!(b.create(&e).is_err());
    }

    #[test]
    fn end_tree_ends_descendants_first() {
        let (_d, b) = board();
        let root = b.reserve_root().unwrap();
        let c = b.reserve_child(&root).unwrap();
        let g = b.reserve_child(&c).unwrap();
        for s in [&root, &c, &g] {
            b.create(&Entry::new(Status::Working, s.clone(), "claude", "w"))
                .unwrap();
        }
        let other = b.reserve_root().unwrap();
        b.create(&Entry::new(Status::Working, other.clone(), "claude", "o"))
            .unwrap();
        assert_eq!(b.end_tree(&root, Duration::ZERO).unwrap(), 3);
        let live = b.entries().unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].sid, other);
    }

    #[test]
    fn started_at_is_fixed_at_reservation() {
        let (_d, b) = board();
        let sid = b.reserve_root().unwrap();
        let t0 = b.started_at(&sid).expect("started meta");
        std::thread::sleep(Duration::from_millis(30));
        b.create(&Entry::new(Status::Idle, sid.clone(), "claude", "w"))
            .unwrap();
        b.update(&sid, Duration::ZERO, |e| {
            e.status = Status::Working;
            true
        })
        .unwrap();
        b.meta_write(&sid, "session", "x").unwrap();
        b.meta_remove(&sid, "waiting");
        let dir_mtime = fs::metadata(b.sid_dir(&sid)).unwrap().modified().unwrap();
        assert!(dir_mtime > t0, "sid dir mtime moved; started_at must not");
        assert_eq!(b.started_at(&sid), Some(t0));
        let child = b.reserve_child(&sid).unwrap();
        assert!(b.started_at(&child).unwrap() > t0);
    }

    #[test]
    fn end_tree_reports_root_failure_but_continues_children() {
        let (_d, b) = board();
        let root = b.reserve_root().unwrap();
        let c = b.reserve_child(&root).unwrap();
        for s in [&root, &c] {
            b.create(&Entry::new(Status::Working, s.clone(), "claude", "w"))
                .unwrap();
        }
        fs::create_dir(b.sid_dir(&root).join("lock")).unwrap(); // stale lock on root only
        let err = b.end_tree(&root, Duration::ZERO).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        let live = b.entries().unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].sid, root);
        assert_eq!(b.ended().unwrap()[0].sid, c);
    }

    #[test]
    fn release_removes_reservation() {
        let (_d, b) = board();
        let sid = b.reserve_root().unwrap();
        b.meta_write(&sid, "agent_id", "x").unwrap();
        b.release(&sid).unwrap();
        assert!(!b.has_sid(&sid));
        assert_eq!(b.find_by_meta("agent_id", "x").unwrap(), None);
    }

    #[test]
    fn meta_round_trip() {
        let (_d, b) = board();
        let sid = b.reserve_root().unwrap();
        assert_eq!(b.meta_read(&sid, "agent_id"), None);
        b.meta_write(&sid, "agent_id", "abc\n").unwrap();
        assert_eq!(b.meta_read(&sid, "agent_id").as_deref(), Some("abc"));
        assert_eq!(
            b.find_by_meta("agent_id", "abc").unwrap(),
            Some(sid.clone())
        );
        assert_eq!(b.find_by_meta("agent_id", "zzz").unwrap(), None);
        b.meta_remove(&sid, "agent_id");
        assert_eq!(b.meta_read(&sid, "agent_id"), None);
    }
}
