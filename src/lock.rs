//! `mkdir .sids/<sid>/lock` によるロック。古いロックは消さない（lint が報告する）。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_WAIT: Duration = Duration::from_secs(2);
pub const HOOK_WAIT: Duration = Duration::from_millis(500);
const RETRY: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
}

pub fn acquire(sid_dir: &Path, wait: Duration) -> io::Result<Lock> {
    let path = sid_dir.join("lock");
    let deadline = Instant::now() + wait;
    loop {
        match fs::create_dir(&path) {
            Ok(()) => return Ok(Lock { path }),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        format!("lock is held: {}", path.display()),
                    ));
                }
                thread::sleep(RETRY);
            }
            Err(e) => return Err(e),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_released() {
        let dir = tempfile::tempdir().unwrap();
        let l1 = acquire(dir.path(), Duration::from_millis(0)).unwrap();
        let err = acquire(dir.path(), Duration::from_millis(150)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        drop(l1);
        let _l2 = acquire(dir.path(), Duration::from_millis(0)).unwrap();
    }

    #[test]
    fn missing_sid_dir_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(acquire(&dir.path().join("nope"), Duration::ZERO).is_err());
    }
}
