# ロードマップ

## v1 の範囲（公開済み）

- macOS / Linux のローカルの board 1 つ
- Claude Code の hook（サブエージェントを含む）
- コマンド: `init` `run` `hook claude` `set` `ls` `wait` `watch` `end` `lint`
- `wait` のブロッキング待機と、1 秒窓でのまとめ、注意が要る遷移だけの通知
- GitHub Actions: CI（fmt / clippy / test を ubuntu と macOS で）、タグ `v*` で aarch64-apple-darwin、aarch64-unknown-linux-musl、x86_64-unknown-linux-musl のバイナリを添付したドラフトリリース

## 将来課題

優先順ではなく、着手の目安を添える。

1. **docker / VM の中のエージェント。** 中に Linux バイナリを置き、hook が中の board に書く。ホストから `--via "limactl shell vm"` や `--via "docker exec c"` のように board を読む。複数 board のまとめ表示（行の先頭に出どころを付ける）。sid は board ごとに独立なので、家族は board ごとに見る
2. **他エージェントのアダプタ。** Copilot CLI（プロトタイプで検証済み、`--plugin-dir`）、agy（常設登録、承認待ちは推測）、Codex、Gemini。手順は [adapters.md](adapters.md)。Copilot が最初の候補
3. **`lsboard last SID`。** `transcript` の meta から最後の発言の抜粋を出す。試用で「最後の発言を読む機能は便利だった」とされたもの
4. **Claude Code 以外のエージェントを起こす手段。** `watch` の出力を herdr や tmux のキー送信につなぐ中継。Claude Code はバックグラウンド Bash の `wait` で足りるので、lsboard の外に置くか薄い補助にとどめる
5. **複数の親が同時に `wait` するときの重複通知の整理。** 同じ子を複数の `wait` が見ると、それぞれに届く。問題になってから考える
6. **Windows。** rename の原子性と `mkdir` ロックは動くはずだが未確認。シグナル処理は `#[cfg(not(unix))]` で空実装
7. **公開前のチェック。** 業務由来に当たらないかの確認、商標（J-PlatPat / USPTO）、ライセンス表記の確認、`notes/prototype/tests/fixtures/` の匿名化（公開するなら）
8. **英語化と org への移管。** [decisions.md](decisions.md) の基準（2 つ目のアダプタ、または外部からの反応）を満たしたら、README と docs を英語にし、allpaqa-org へ転送する
9. **`actions/checkout` を v5 に上げる。** v4 が Node 20 依存で非推奨の警告が出ている
10. **実セッションでの確認。** 対話モードの PermissionRequest（承認ダイアログ）と SubagentStart / SubagentStop を、v1 の参考実装を通した本物のセッションで確かめる。`/clear` と `/resume` の SessionEnd の `reason` も記録する
