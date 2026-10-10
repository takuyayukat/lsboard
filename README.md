# lsboard

> **ls で見える。wait で眠れる。**

並列に動く AI エージェント（Claude Code とそのサブエージェントなど）の状態を、1 作業者 1 ファイルのファイル名に表すボードです。親エージェントは `lsboard wait` で子の変化を**トークンを使わずに**待てます。

```console
$ ls ~/.local/state/lsboard/board
10-waiting__k3f9.01__claude__explorer approve Bash
20-working__k3f9__claude__conductor
40-idle__m7q2__claude__worker
```

- entry は空ファイルで、ファイル名が正本です。更新は rename だけ
- status は `10-waiting` `20-working` `30-background` `40-idle` `90-ended`。番号が小さいほど人の注意が要り、`ls` で上に並びます
- デーモンは要りません。`wait` と `watch` がその場でディレクトリを見張ります

## インストール

```sh
cargo install --path .
```

Linux（docker / VM の中）向けのバイナリは GitHub Releases のタグビルドで作られます。

## 使い方

```sh
lsboard init                                  # ~/.local/state/lsboard/board を作る（$LSBOARD_DIR で変更）
lsboard run -- claude                         # entry を作り、hook を登録して Claude Code を起動する
lsboard run --name worker --parent k3f9 -- claude   # k3f9 の子として起動する
lsboard run --agent claude -- ./my-claude-wrapper   # ラッパー経由でも claude の hook を登録する
lsboard ls --tree                             # 整形して見る（生の ls でも読める）
lsboard wait                                  # 子のどれかが waiting / idle / ended になるまでブロックする
lsboard watch                                 # 変化を 1 行ずつ流し続ける
lsboard set waiting "merge PR 100"            # エージェント自身の宣言（hook は解除しない）
lsboard lint                                  # 不整合の報告
```

`run` で起動したエージェントには `LSBOARD_SID` と `LSBOARD_DIR` が渡り、Claude Code には `--settings` で hook が登録されます。hook はこの 2 つの環境変数がないと何もしません。

### 親が子を待つ

```sh
# conductor の中で。バックグラウンドに置けば、戻ったときに 1 回だけ起こされる
lsboard wait
# → k3f9.02 10-waiting worker approve merge
```

出力は 1 行 1 entry（`<sid> <status> <name> [<message>]`）。`--json` で機械向けにもなります。`--all` で全員が条件を満たすまで待ち、`--timeout SEC` を過ぎると終了コード 2 で戻ります。すでに条件を満たしていればすぐ戻ります。

## 構成

```
board/
├── .lsboard          # 版の印
├── .sids/<sid>/      # sid の予約（mkdir）、lock/、agent、pid、session、waiting など
├── .ended/           # 終わった entry
└── <entry>           # {status}__{sid}__{agent}__{name}[ {message}]
```

sid は親子を表します（`k3f9` → `k3f9.01` → `k3f9.01.02`）。

## 既知の限界

- Claude Code で承認を拒否すると、PostToolUse も Stop も発火しないため、待ちの表示は次のプロンプト送信まで残ります
- 承認したツールが失敗したときや、承認ダイアログで入力を書き換えたときは、PostToolUse のハッシュが一致しないため、待ちの表示は Stop か次のプロンプト送信まで残ります
- SessionEnd の hook で自分の entry を終わらせるのは `prompt_input_exit` と `logout` のときだけです。`/clear` などセッションが続く理由では触るだけで、サブエージェントの子だけを終わらせます。プロセスが本当に終わったときの片付けは `lsboard run` が行います
- `lsboard run` に TERM / HUP が来ると子に TERM を転送し、子が終わってから entry を片付けます。Ctrl-C は子に任せます（親は無視します）
- v1 は macOS / Linux のローカルの board だけを対象にします。docker / VM の中の board を外から読む仕組み、Claude Code 以外の hook は将来の課題です

## 開発

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## ライセンス

MIT
