//! sid: 作業者の ID。親子関係を含む（`k3f9` → `k3f9.01` → `k3f9.01.02`）。

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

const B36: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const ROOT_LEN: usize = 4;
const CHILD_LEN: usize = 2;
const MAX_CHILDREN: u32 = 36 * 36 - 1;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sid(String);

fn is_b36(c: char) -> bool {
    c.is_ascii_digit() || c.is_ascii_lowercase()
}

impl Sid {
    pub fn parse(s: &str) -> Option<Sid> {
        let mut parts = s.split('.');
        let root = parts.next()?;
        if root.chars().count() != ROOT_LEN || !root.chars().all(is_b36) {
            return None;
        }
        for p in parts {
            if p.chars().count() != CHILD_LEN || !p.chars().all(is_b36) {
                return None;
            }
        }
        Some(Sid(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn depth(&self) -> usize {
        self.0.matches('.').count()
    }

    pub fn parent(&self) -> Option<Sid> {
        self.0.rsplit_once('.').map(|(p, _)| Sid(p.to_string()))
    }

    pub fn is_child_of(&self, parent: &Sid) -> bool {
        self.parent().as_ref() == Some(parent)
    }

    pub fn is_descendant_of(&self, ancestor: &Sid) -> bool {
        self.0.len() > ancestor.0.len()
            && self.0.starts_with(&ancestor.0)
            && self.0.as_bytes()[ancestor.0.len()] == b'.'
    }

    fn child(&self, n: u32) -> Sid {
        let hi = B36[(n / 36) as usize] as char;
        let lo = B36[(n % 36) as usize] as char;
        Sid(format!("{}.{hi}{lo}", self.0))
    }
}

impl fmt::Display for Sid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Sid {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Sid::parse(s).ok_or_else(|| format!("invalid sid: {s}"))
    }
}

/// 外部 crate なしの小さな乱数。sid の衝突は mkdir が検出するので、品質は問わない。
struct Rng(u64);

impl Rng {
    fn new() -> Rng {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        let seed = nanos ^ (u64::from(std::process::id()) << 32) ^ 0xd1b5_4a32_d192_ed03;
        Rng(if seed == 0 { 1 } else { seed })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn b36(&mut self, len: usize) -> String {
        (0..len)
            .map(|_| B36[(self.next() % 36) as usize] as char)
            .collect()
    }
}

/// 新しい root の sid を `mkdir sids_dir/<sid>` で予約する。
pub fn reserve_root(sids_dir: &Path) -> io::Result<Sid> {
    let mut rng = Rng::new();
    for _ in 0..1000 {
        let s = rng.b36(ROOT_LEN);
        match fs::create_dir(sids_dir.join(&s)) {
            Ok(()) => return Ok(Sid(s)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(
        "could not reserve a root sid after 1000 tries",
    ))
}

/// 親の下に、01 から順に空いている番号の子を予約する。
pub fn reserve_child(sids_dir: &Path, parent: &Sid) -> io::Result<Sid> {
    if !sids_dir.join(parent.as_str()).is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("parent sid is not reserved on this board: {parent}"),
        ));
    }
    for n in 1..=MAX_CHILDREN {
        let child = parent.child(n);
        match fs::create_dir(sids_dir.join(child.as_str())) {
            Ok(()) => return Ok(child),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(format!(
        "no child sid left under {parent}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_and_rejects_invalid() {
        assert!(Sid::parse("k3f9").is_some());
        assert!(Sid::parse("k3f9.01").is_some());
        assert!(Sid::parse("k3f9.01.zz").is_some());
        assert!(Sid::parse("K3F9").is_none());
        assert!(Sid::parse("k3f").is_none());
        assert!(Sid::parse("k3f9.1").is_none());
        assert!(Sid::parse("k3f9..01").is_none());
        assert!(Sid::parse("k3f9_01").is_none());
        assert!(Sid::parse("").is_none());
    }

    #[test]
    fn family_relations() {
        let root = Sid::parse("k3f9").unwrap();
        let c1 = Sid::parse("k3f9.01").unwrap();
        let g1 = Sid::parse("k3f9.01.02").unwrap();
        let other = Sid::parse("k3f91").map(|_| ());
        assert!(other.is_none());
        assert_eq!(root.depth(), 0);
        assert_eq!(g1.depth(), 2);
        assert_eq!(c1.parent(), Some(root.clone()));
        assert_eq!(root.parent(), None);
        assert!(c1.is_child_of(&root));
        assert!(!g1.is_child_of(&root));
        assert!(g1.is_descendant_of(&root));
        assert!(!root.is_descendant_of(&c1));
        assert_eq!(root.child(1).as_str(), "k3f9.01");
        assert_eq!(root.child(36).as_str(), "k3f9.10");
        assert_eq!(root.child(MAX_CHILDREN).as_str(), "k3f9.zz");
    }

    #[test]
    fn reservation_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let root = reserve_root(dir.path()).unwrap();
        assert!(dir.path().join(root.as_str()).is_dir());
        let c1 = reserve_child(dir.path(), &root).unwrap();
        let c2 = reserve_child(dir.path(), &root).unwrap();
        assert_eq!(c1.as_str(), format!("{root}.01"));
        assert_eq!(c2.as_str(), format!("{root}.02"));
        let missing = Sid::parse("zzzz").unwrap();
        assert!(reserve_child(dir.path(), &missing).is_err());
    }
}
