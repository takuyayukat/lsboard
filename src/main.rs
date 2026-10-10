//! lsboard — ls で見える、wait で眠れる AI エージェントの状態ボード。

mod board;
mod entry;
mod hooks;
mod lint;
mod lock;
mod run;
mod sid;
mod status;
mod sys;
mod view;
mod watch;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use board::Board;
use lock::DEFAULT_WAIT;
use sid::Sid;
use status::Status;

#[derive(Parser)]
#[command(
    name = "lsboard",
    version,
    about = "ls で見える、wait で眠れる AI エージェントの状態ボード"
)]
struct Cli {
    /// board のディレクトリ（既定: $LSBOARD_DIR、なければ ~/.local/state/lsboard/board）
    #[arg(long, global = true, env = "LSBOARD_DIR", value_name = "DIR")]
    board: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// board を作る
    Init {
        /// 作る場所（省略時は --board か既定の場所）
        dir: Option<PathBuf>,
    },
    /// entry を作り、hook を登録してエージェントを起動する
    Run(run::RunArgs),
    /// エージェントの hook の入口（標準入力に JSON）。常に 0 で終わる
    Hook {
        /// エージェントの種類（claude）
        agent: String,
    },
    /// 自分の状態を宣言する
    Set {
        /// 対象の sid（既定: $LSBOARD_SID）
        #[arg(long, env = "LSBOARD_SID")]
        sid: Option<String>,
        #[command(subcommand)]
        what: SetCmd,
    },
    /// entry を終わらせて .ended/ に移す（子孫も）
    End { sid: String },
    /// 整形して見る
    Ls {
        /// sid の親子で木にする
        #[arg(long)]
        tree: bool,
        /// JSON で出す
        #[arg(long)]
        json: bool,
    },
    /// 子の変化を待つ（ブロックする）
    Wait(watch::WaitArgs),
    /// 変化を 1 行ずつ流し続ける
    Watch(watch::WatchArgs),
    /// 不整合を報告する（あれば終了コード 1）
    Lint,
}

#[derive(Subcommand)]
enum SetCmd {
    /// 人の対応が要る。message に何が要るかを書く
    Waiting { message: String },
    /// 処理中に戻す（待ちを解除する）
    Working,
    /// 手が空いた（待ちを解除する）
    Idle,
}

fn sid_arg(s: Option<String>) -> Result<Sid, String> {
    let s = s.ok_or("sid is required (--sid or LSBOARD_SID)")?;
    Sid::parse(&s).ok_or_else(|| format!("invalid sid: {s}"))
}

fn set(board_arg: Option<PathBuf>, sid: Option<String>, what: SetCmd) -> Result<u8, String> {
    let board = Board::open(board_arg).map_err(|e| e.to_string())?;
    let sid = sid_arg(sid)?;
    let (status, message) = match what {
        SetCmd::Waiting { message } => (Status::Waiting, entry::sanitize_message(&message)),
        SetCmd::Working => (Status::Working, String::new()),
        SetCmd::Idle => (Status::Idle, String::new()),
    };
    let updated = board
        .update(&sid, DEFAULT_WAIT, |e| {
            e.status = status;
            e.message = message;
            true
        })
        .map_err(|e| e.to_string())?;
    match updated {
        None => Err(format!("no entry for sid {sid}")),
        Some(_) => {
            if status == Status::Waiting {
                board
                    .meta_write(&sid, "waiting", "agent")
                    .map_err(|e| e.to_string())?;
            } else {
                board.meta_remove(&sid, "waiting");
            }
            Ok(0)
        }
    }
}

fn dispatch(cli: Cli) -> Result<u8, String> {
    match cli.cmd {
        Cmd::Init { dir } => {
            let b = Board::init(dir.or(cli.board)).map_err(|e| e.to_string())?;
            println!("{}", b.dir.display());
            Ok(0)
        }
        Cmd::Run(args) => run::run(cli.board, args),
        Cmd::Hook { agent } => Ok(hooks::run(&agent)),
        Cmd::Set { sid, what } => set(cli.board, sid, what),
        Cmd::End { sid } => {
            let board = Board::open(cli.board).map_err(|e| e.to_string())?;
            let sid = sid_arg(Some(sid))?;
            let n = board
                .end_tree(&sid, DEFAULT_WAIT)
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err(format!("no entry for sid {sid}"));
            }
            Ok(0)
        }
        Cmd::Ls { tree, json } => view::ls(cli.board, tree, json),
        Cmd::Wait(args) => watch::wait(cli.board, args),
        Cmd::Watch(args) => watch::watch(cli.board, args),
        Cmd::Lint => lint::lint(cli.board),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("lsboard: {e}");
            ExitCode::from(1)
        }
    }
}
