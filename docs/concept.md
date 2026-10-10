# コンセプト

> **ls で見える。wait で眠れる。**

## 何を解くか

並列に動く AI エージェント（Claude Code とそのサブエージェント、別プロセスとして起動した worker など）について、次の 2 つを安く実現する。

1. **見る**: 誰が人を待っていて、誰が動いていて、誰が終わったかを `ls` 1 回で知る
2. **起こす**: 親エージェントが、子の変化を LLM のトークンを使わずに待ち、注意が要る変化だけで起きる

対象はローカルのマシンで動くエージェント。docker や VM の中のエージェントは将来の課題として [roadmap.md](roadmap.md) に置く。

**背景**: 複数のエージェントを束ねる親（orkestrado の conductor など）は、子の状態を知るために監視イベントを受け取るたびに LLM で評価していた。これがトークン消費の最大の要因だった。lsboard は、待っている間はプロセスとして眠り、起きたときに読むのは数行、という形でこの消費をなくす。

## 仕組みの骨格

- 1 作業者 1 ファイル。ファイルは空で、**ファイル名が正本**
- ファイル名の先頭に番号つきの status を置く。`ls` の並び順がそのまま「人の注意が要る順」になる
- 更新は rename だけ。読み書きの競合がない
- 親子は sid（`k3f9` → `k3f9.01`）で表す。`sort` で家族がまとまる
- `wait` は条件が満たされるまでブロックし、満たした entry を 1 行ずつ出して戻る。`watch` は同じ変化を流し続ける
- デーモンは正本を持たない。`wait` と `watch` はその場でディレクトリを見張る読み手にすぎない

```console
$ ls ~/.local/state/lsboard/board
10-waiting__k3f9.01__claude__explorer approve Bash
20-working__k3f9__claude__conductor
40-idle__m7q2__claude__worker
```

## 先例

状態をファイル名に持ち、rename で原子的に反映する骨格は Maildir（1995 年、D. J. Bernstein）と同じである。ディレクトリを `ls` してジョブを見る点は nq、規約そのものが本体で実装は参考という立て付けは todo.txt に近い。lsboard はこれらの古典的な手法を、AI エージェントの hook イベントと、親子関係を持つ作業者と、親を安く起こすという今の問題に当てはめたものである。保存の仕組みに新しさはなく、その上に載せたものが貢献である。

## やらないこと

- タスクの計画やバックログの管理（GitHub Issues などに任せる）
- ワークフローの段階（step、repo、task、role）の管理。プロトタイプではこれが複雑さの主因だった
- 外部サービスへの書き込み
- 書き手が生きているかをファイルに書くこと（読み手と lint が導く）
- board をまたいだ親子関係
- 承認そのもの。message は目印であり、承認はエージェントの画面で内容を確かめてから行う

## 用語

| 用語 | 意味 |
|---|---|
| board | entry を置くディレクトリ。既定は `~/.local/state/lsboard/board` |
| entry | board 直下の空ファイル。1 作業者に 1 つ。ファイル名が状態を表す |
| writer | entry を作って更新する側。`lsboard run`、hook、`lsboard set` |
| reader | board を読む側。`ls`、`lsboard ls`、`wait`、`watch`、lint |
| sid | 作業者の ID。親子関係を含む（`k3f9.01` は `k3f9` の子） |
| 家族 | 一番上の親（root）と、その子孫 |
| meta | `.sids/<sid>/` に置く小さなファイル。ファイル名に出さない付随情報（pid、セッション ID など） |
| adapter | エージェントの hook 入力を lsboard の状態遷移に変換するモジュール。`lsboard hook <agent>` の中身 |
| hook | エージェントがイベントのたびに呼ぶ外部コマンド。lsboard では `lsboard hook claude` を登録する |
| 宣言された待ち | エージェント自身が `lsboard set waiting` で付けた `10-waiting`。hook は触らない |
| hook 由来の待ち | 承認要求の hook が付けた `10-waiting`。対応するツールの実行後に hook が解除する |

名前はコードと文書で同じものを使う。コードでは `Board` `Entry` `Sid` `Status` と、hook モジュール `hooks::claude` がこれに対応する。
