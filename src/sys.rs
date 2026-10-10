//! OS との薄い境界。libc crate を足さずに済むよう、必要な関数だけを宣言する。

#[cfg(unix)]
mod imp {
    use std::io;
    use std::process::Command;
    use std::sync::atomic::{AtomicI32, Ordering};

    pub const SIGHUP: i32 = 1;
    pub const SIGINT: i32 = 2;
    pub const SIGTERM: i32 = 15;
    const SIG_DFL: usize = 0;
    const SIG_IGN: usize = 1;
    const ESRCH: i32 = 3;

    unsafe extern "C" {
        fn signal(sig: i32, handler: usize) -> usize;
        fn kill(pid: i32, sig: i32) -> i32;
    }

    static RECEIVED: AtomicI32 = AtomicI32::new(0);

    extern "C" fn on_signal(sig: i32) {
        RECEIVED.store(sig, Ordering::Relaxed);
    }

    /// 親（`lsboard run`）側: Ctrl-C は子に任せて無視し、TERM / HUP は記録だけする。
    pub fn install_parent_handlers() {
        // SAFETY: 標準の signal(2)。ハンドラは atomic への store しかしない。
        unsafe {
            signal(SIGINT, SIG_IGN);
            signal(SIGTERM, on_signal as *const () as usize);
            signal(SIGHUP, on_signal as *const () as usize);
        }
    }

    /// 子では既定の挙動に戻す（SIG_IGN は exec を越えて引き継がれるため）。
    pub fn reset_in_child(cmd: &mut Command) {
        use std::os::unix::process::CommandExt;
        // SAFETY: fork 後 exec 前に async-signal-safe な signal(2) を呼ぶだけ。
        unsafe {
            cmd.pre_exec(|| {
                signal(SIGINT, SIG_DFL);
                signal(SIGTERM, SIG_DFL);
                signal(SIGHUP, SIG_DFL);
                Ok(())
            });
        }
    }

    /// 親が受け取ったシグナル番号（なければ 0）。
    pub fn received_signal() -> i32 {
        RECEIVED.load(Ordering::Relaxed)
    }

    pub fn terminate(pid: u32) {
        // SAFETY: kill(2) に自分の子の pid を渡すだけ。
        unsafe {
            kill(pid as i32, SIGTERM);
        }
    }

    /// プロセスが存在するか。権限がなくて見えない場合は「いる」とみなす。
    pub fn process_exists(pid: u32) -> bool {
        // SAFETY: シグナル 0 の kill(2) は存在確認だけを行う。
        let rc = unsafe { kill(pid as i32, 0) };
        rc == 0 || io::Error::last_os_error().raw_os_error() != Some(ESRCH)
    }

    pub fn exit_code_of(status: &std::process::ExitStatus) -> u8 {
        use std::os::unix::process::ExitStatusExt;
        if let Some(c) = status.code() {
            c.clamp(0, 255) as u8
        } else if let Some(sig) = status.signal() {
            (128 + sig).clamp(0, 255) as u8
        } else {
            1
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use std::process::Command;

    pub fn install_parent_handlers() {}
    pub fn reset_in_child(_cmd: &mut Command) {}
    pub fn received_signal() -> i32 {
        0
    }
    pub fn terminate(_pid: u32) {}
    pub fn process_exists(_pid: u32) -> bool {
        true
    }
    pub fn exit_code_of(status: &std::process::ExitStatus) -> u8 {
        status.code().unwrap_or(1).clamp(0, 255) as u8
    }
}

pub use imp::*;
