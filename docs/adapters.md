# アダプタの追加

アダプタは、あるエージェントの hook 入力を lsboard の状態遷移に変換するモジュールである。v1 に入っているのは Claude Code（[hooks/claude.md](hooks/claude.md)）だけ。他のエージェントは、ここに書く手順で足す。

## 手順

1. **遷移表を書く。** [hooks/](hooks/) に `<agent>.md` を作り、そのエージェントのイベントごとに lsboard の遷移を表にする。Claude Code の表を雛形にする。イベントの入力の項目名は、実機から採取した JSON で確かめる
2. **モジュールを書く。** `src/hooks/<agent>.rs` に `pub fn handle(ctx: &Ctx, v: &Value) -> Result<(), String>` を書き、`src/hooks/mod.rs` の `match agent` に 1 行足す。これで `lsboard hook <agent>` が動く。遷移は `Status` の enum と網羅的な `match` で書き、抜けをコンパイルで検出する
3. **登録方法を足す。** `src/run.rs` で、`agent` がそのエージェントのときに hook の設定を生成して起動引数に差し込む（Claude Code の `--settings` に相当するもの）。起動ごとの登録ができないエージェントは、利用者の常設の設定に書く手順を文書にする。hook は `LSBOARD_SID` がなければ何もしないので、常設でも普段の起動には影響しない
4. **テストを書く。** 実機から採取した hook の入力を fixture にして `tests/cli.rs` から `lsboard hook <agent>` に流し、各イベントの後のファイル名を assert する。fixture は公開前に手元のパスとセッション ID を匿名化する
5. **`run --agent` を確かめる。** ラッパー経由で起動しても `--agent <agent>` でアダプタが選ばれること

## アダプタが守ること

- `LSBOARD_SID` と `LSBOARD_DIR` がなければ何もしない。常に終了コード 0 で終わる
- 自分と、自分の hook が作った子の entry 以外を書かない
- エージェントが宣言した待ち（`waiting` が `agent`）を上書きも解除もしない。解除するのは人のプロンプト送信だけ
- 待ちを抜けるときは message と `waiting` の meta を消す
- 迷ったら解除しない
- 自分のセッションが知っていることだけで判断する。他の entry を読んで推測しない
- セッションが続く可能性のあるイベントで自分の entry を終わらせない。本当の終了は `lsboard run` の後始末に任せる

## hook のないエージェント

`lsboard run --no-hooks -- <cmd>` で起動すれば、起動（`40-idle`）と終了（`90-ended`）だけは board に載る。エージェント自身に `lsboard set waiting "..."` などを呼ばせれば、宣言だけで運用できる。`--no-hooks` を付けずにアダプタのないエージェントを起動すると、stderr に 1 行の注意が出る。

## 候補と分かっていること

以下はプロトタイプ（bash 実装）で実機を確かめた記録で、v1 では未実装。イベント名や入力の形は当時の版のもので、実装前に再確認する。

### Copilot CLI（1.0.94 で確認）

- **起動ごとの登録**: `--plugin-dir <dir>` に `plugin.json` と `hooks.json` を置くと、そのセッションだけで hook が有効になる
- イベント名を PascalCase で登録すると、多くのイベントの入力が Claude Code とほぼ同じ形（`session_id` `tool_name` `tool_input`）になる。ただし PermissionRequest と SubagentStart の入力だけ camelCase（`sessionId` `toolName`）
- PermissionRequest は承認が要らない設定（`--allow-all-tools`）でも発火する。人に確認を求めるときだけ発火するのは Notification の `permission_prompt`。質問のツールでは Notification の `elicitation_dialog` が発火する。承認待ちの検出には Notification を使い、PermissionRequest は使わない
- サブエージェントは別の `session_id` を持つ。SubagentStop の `agent_id` はサブエージェントの `session_id` と同じ。自分の `session_id` と違うイベントは子のものとして扱い、まだ結び付いていない子のうち一番新しいものに結び付ける（並列では間違えることがある）
- Stop に `background_tasks` はない
- `-p` では最初の UserPromptSubmit が SessionStart より先に来る
- 承認を拒否すると、ツール実行後の hook もターン終了の hook も発火しない（Claude Code と同じ）

### Antigravity CLI（agy。1.3.2 / 1.3.3 で確認）

- **起動ごとの登録方法がない。** 作業ディレクトリの `.agents/hooks.json` は対象のリポジトリを汚すので使わない。ユーザー全体の `~/.gemini/config/hooks.json` に常設する。他のツールも同じファイルを使うので、導入では確認を取ってから項目を足す
- イベントは PreToolUse / PostToolUse / PreInvocation / PostInvocation / Stop の 5 つだけ。承認の要求、セッションの開始と終了、サブエージェントのイベントはない
- PreInvocation と PostInvocation はターンの中でモデルを呼ぶたびに発火し、人がプロンプトを送ると `invocationNum` が 0 に戻る。これを「新しいプロンプト」の合図にする
- PreToolUse は承認ダイアログの前に発火し、ダイアログの間は何も発火しない。拒否（Esc）と `/exit` ではどのイベントも発火しない
- ツールの hook は `matcher` を付けないと発火しない
- 入力は全イベントに `conversationId`、ツールの hook に `toolCall`（`name` と `args`）、Stop に `fullyIdle` と `terminationReason`
- 承認待ちを hook で付けられないので、PreToolUse でツール名を meta に書き、PostToolUse で消し、読む側が「meta が残っていて mtime が古い」ことから承認待ちか長いツールの実行中かを推測する（PENDING）
- セッション開始のイベントがないので context の注入はできない。宣言の仕方は AGENTS.md などで伝える

### Codex CLI / Gemini CLI

未確認。Codex は `codex -c 'hooks.<Event>=[...]'` で起動ごとに指定できるとの報告がある。Gemini CLI は settings.json の hooks を持つが、起動ごとの指定方法は未確認。
