# Claude Code の hook

Claude Code のイベントを lsboard の状態遷移に変換するアダプタ。実装は `src/hooks/claude.rs`。

## 登録

`lsboard run -- claude ...` が、起動のたびに次を行う。

1. `.sids/<sid>/settings.json` に hook の設定を書く。コマンドは `lsboard` 自身の絶対パス（`current_exe()`）を単一引用符で囲んだ `'<path>' hook claude`
2. `claude` の引数の先頭に `--settings <そのパス>` を差し込む
3. 子の環境に `LSBOARD_SID` と `LSBOARD_DIR` を入れる
4. 終了時に `settings.json` を消す

登録するイベントは 9 つ。ツールに関する 3 つ（PreToolUse、PermissionRequest、PostToolUse）には `"matcher": "*"` を付ける。

```
SessionStart  UserPromptSubmit  PreToolUse  PermissionRequest  PostToolUse
Stop  SubagentStart  SubagentStop  SessionEnd
```

`--settings` の hook はユーザー設定の hook と合成される（実機で確認済み）。利用者の常設の設定は変えない。ラッパースクリプト経由で起動するときは `lsboard run --agent claude -- ./wrapper` と指定する。

## 遷移表

入力は標準入力の JSON。`hook_event_name` で分岐する。「対象」は原則として自分（`LSBOARD_SID`）の entry だが、入力に `agent_id` があり、その値を `agent_id` の meta に持つ子がいれば、その子の entry が対象になる。

| hook | 遷移 |
|---|---|
| SessionStart | `session_id` を meta `session` に、`transcript_path` を meta `transcript` に書く。context の説明文を標準出力に出す（下記） |
| UserPromptSubmit | `prompt` が `<task-notification>` で始まるなら自動の通知: `40-idle` か `30-background` のときだけ → `20-working`。待ちは解除しない。それ以外は人の入力: → `20-working`、message を消し、`waiting` の meta を消す（宣言された待ちも解除する） |
| PreToolUse | `40-idle` か `30-background` なら → `20-working`。それ以外は mtime の更新だけ。待ちは解除しない |
| PermissionRequest | `waiting` が `agent` なら mtime の更新だけ。そうでなければ → `10-waiting`、message は `approve <tool_name>`（AskUserQuestion なら `question`）、meta `waiting` に `hook <ハッシュ>` を書く |
| PostToolUse | `waiting` が `hook <ハッシュ>` で、ツール名と入力から計算したハッシュが一致すれば → `20-working`、message と `waiting` を消す。それ以外は mtime の更新だけ |
| Stop | `background_tasks` に `status` が `running` のものがあれば、`20-working` → `30-background` にして終わり。なければ `20-working` と `30-background` → `40-idle`、さらに hook 由来の `10-waiting` も → `40-idle`（message と `waiting` を消す）。宣言された待ちはどちらの場合も触らない |
| SubagentStart | `agent_id` がなければ何もしない。すでに同じ `agent_id` の子がいれば何もしない。そうでなければ子の sid を予約し、meta `agent`（親と同じ）、`agent_id`、`name`（`agent_type` を token にして 64 文字に切ったもの。なければ `subagent`）、`pid`（親と同じ）を書き、`20-working` で entry を作る。失敗したら予約ごと消す |
| SubagentStop | `agent_id` に対応する子の entry を終わらせる |
| SessionEnd | この hook が作った子（`agent_id` の meta を持つ子孫）をどの `reason` でも終わらせる。自分を終わらせるのは `reason` が `prompt_input_exit` か `logout` のときだけ。それ以外（`clear` `bypass_permissions_disabled` `other`）は mtime の更新だけ |
| Notification | 登録しない。承認の要求は PermissionRequest で拾う |

**SessionEnd を保守的にしている理由**: `reason` の一覧は `clear` `logout` `prompt_input_exit` `bypass_permissions_disabled` `other` で、プロセスが本当に終わるのは `prompt_input_exit` と `logout` だけである。`/clear` やその他の理由で自分の entry を終わらせると、セッションが続いているのに entry が消え、以後の hook がすべて空振りする。本当の終了は `lsboard run` の後始末（`end_tree`）が拾うので、hook 側は迷ったら終わらせない。

## context の注入

SessionStart の標準出力は Claude Code が context に加える。出す文面は次の 5 行。

```
lsboard: you are sid k3f9 on board /Users/me/.local/state/lsboard/board.
- Declare a human-attention wait:  lsboard set waiting "what you need"
- Back to work / idle:             lsboard set working | lsboard set idle
- Wait for your children cheaply:  lsboard wait   (blocks; run it in the background)
- See everyone:                    lsboard ls --tree
```

## ツールのハッシュ

承認の要求（PermissionRequest）とツール実行後（PostToolUse）で同じ値になる識別子。PermissionRequest には `tool_use_id` が入らないので、名前と入力から作る。

```
sha256( tool_name + "\n" + canonical_json(tool_input) )  の hex 64 桁
```

- `tool_input` の最上位から `answers` と `annotations` を除いてから直列化する。AskUserQuestion は回答後にこの 2 つが入力に加わるため
- `tool_input` が null なら `{}` として扱う
- 鍵の順序は辞書順に正規化する。参考実装は serde_json の既定（`Map` が BTreeMap）に頼っているので、`preserve_order` feature を有効にしてはならない

## サブエージェントの扱い

- サブエージェントの中で起きた hook には `agent_id` と `agent_type` が入る。`agent_id` で子の entry を引き、見つからなければ親の entry を対象にする
- SubagentStart で子の sid を `LSBOARD_SID` の子として予約する。`lsboard run --parent` で起動した別プロセスの子と同じ board 上で並ぶ
- SessionEnd で終わらせるのは `agent_id` の meta を持つ子だけ。`--parent` で起動した別プロセスの子は、その子自身の `run` が片付ける

## 実機で確かめたこと

**Claude Code 2.1.285、プロトタイプでの確認**

- `--settings` の hook はユーザー設定の hook と合成される
- サブエージェントの中のツールの hook には `agent_id` と `agent_type` が入る
- PreToolUse と PostToolUse には同じ `tool_use_id` が入るが、PermissionRequest には入らない（ツール名と入力は入る）
- AskUserQuestion も PermissionRequest を発火する。PostToolUse の `tool_input` には `answers` と `annotations` が加わる
- バックグラウンドの作業の完了通知でも UserPromptSubmit が発火する（`prompt` が `<task-notification>` で始まる）
- 承認を拒否すると、PostToolUse も Stop も発火しない
- `-p` の正常終了では SessionEnd の `reason` は `other`

**v1 の参考実装での確認（2026-10-10）**

`lsboard run --name e2e -- claude -p '...' --allowedTools Bash` を実行し、`ls` を 100 ミリ秒ごとに記録した。

```
40-idle__tlfx__claude__e2e        起動直後
20-working__tlfx__claude__e2e     UserPromptSubmit
40-idle__tlfx__claude__e2e        Stop
90-ended__tlfx__claude__e2e       run の後始末で .ended/ へ
```

`lsboard watch` の出力は 3 行（現れた `40-idle`、`40-idle` への遷移、`90-ended` への遷移）。SessionStart で `session` と `transcript` の meta が書かれた。`lsboard lint` は指摘なし。

プロトタイプで実セッションから採取した hook の入力（`-p` と対話の 2 系統、サブエージェントと PermissionRequest と AskUserQuestion を含む）を `lsboard hook claude` に流し、遷移表どおりに動くことを確認した。

## 既知の限界

- 承認を拒否すると PostToolUse も Stop も発火しないため、`10-waiting` の表示は次のプロンプト送信まで残る。エージェントは実際に人の入力を待っているので、表示としては実態に合っている
- 承認したツールが失敗したときや、承認ダイアログで入力を書き換えたときは、PostToolUse のハッシュが一致しないため、待ちの表示は Stop か次のプロンプト送信まで残る
- 追う待ちは最後の 1 件だけ。承認待ちが同時に複数あると取りこぼすことがある
- 並列のサブエージェントで `agent_id` のない hook が来た場合は親の entry を更新する

## 未確認

- 対話モードでの PermissionRequest（承認ダイアログ）と SubagentStart / SubagentStop の実セッションでの発火。プロトタイプの採取データで遷移は確認したが、v1 の参考実装を通した対話セッションではまだ試していない
- `/clear` と `/resume` のときの SessionEnd の `reason`
