# 仕様

board のディレクトリ配置とファイル名の文法、更新の手順を定める。この文書に従えば、別の言語やシェルスクリプトで書いた writer と reader が、Rust の参考実装と同じ board を共有できる。

**MUST** は互換性のために必須、**SHOULD** は推奨を表す。値や既定値は `src/` の参考実装から取っている。文書とコードが食い違えば、コードを直すか文書を直すかをその場で決め、黙って放置しない。

## 1. 環境変数と既定の場所

| 変数 | 内容 |
|---|---|
| `LSBOARD_DIR` | board のパス。`--board DIR` でも指定できる。どちらもなければ `$HOME/.local/state/lsboard/board`（`HOME` がなければ `USERPROFILE`、それもなければカレントディレクトリからの相対） |
| `LSBOARD_SID` | 自分の sid。`lsboard run` が起動したエージェントの環境に入れる。hook と `set`、`wait` の既定値に使う |

`lsboard run` は子プロセスにこの 2 つを渡す。hook は 2 つがそろっていないときは何もしない（MUST。§13）。

## 2. board の構成

```
board/
├── .lsboard            # 版の印。中身は "1" と改行
├── .sids/<sid>/        # sid の予約（ディレクトリの存在が予約）と meta
│   ├── lock/           # 更新ロック（ディレクトリ）
│   ├── started         # 予約時刻（空ファイル。mtime が started_at）
│   └── ...             # §7 の meta
├── .ended/             # 終わった entry の移動先
└── <entry>             # 空ファイル。直下にフラットに並べる
```

- `.lsboard` がないディレクトリは board として開かない（MUST）。`lsboard init [DIR]` が作る。すでにあれば何もしない
- entry 以外のものは `.` で始まる名前にする（MUST）。`ls` で entry だけが見えるようにするため
- board は書き手と同じローカルのファイルシステムに置く（SHOULD）。NFS、sshfs、virtiofs、9p、Dropbox や iCloud の同期フォルダでは rename の原子性と mtime が保証されない

## 3. status

| 値 | 意味 |
|---|---|
| `10-waiting` | 人の対応が要る（承認、質問、エージェント自身の宣言） |
| `20-working` | 処理中 |
| `30-background` | ターンは終わったが、裏の処理（サブエージェント、バックグラウンドのシェル）を待っている。人の対応は要らない |
| `40-idle` | ターンが終わり、要求もない |
| `90-ended` | 終わった。`.ended/` に移る |

- この 5 つ以外の値は不正とし、reader は entry として扱わない（MUST）。`41-idle` のように番号が違うものも不正
- 番号が小さいほど人の注意が要り、`LC_ALL=C ls` で上に並ぶ
- コマンドの引数では `10-waiting` でも `waiting` でも受け付ける

## 4. sid

```
sid   = root ( "." child )*
root  = [0-9a-z]{4}
child = [0-9a-z]{2}
```

- root は base36 の乱数 4 文字（MUST）。子は親の sid に `.` と 2 文字の番号を足す（MUST）。番号は `01` から順に振り、`zz`（1,295）まで。深さに上限はない
- 大文字は使わない（MUST）。大文字と小文字を区別しないファイルシステムがあるため
- **sid は予約してから使う**（MUST）。予約は `mkdir board/.sids/<sid>` で行い、成功したものだけが自分のものになる。root は衝突したら別の乱数で再試行し、子は `01` から順に最初に成功した番号を使う
- **`mkdir -p` は使わない**（MUST）。すでにあっても成功してしまい、予約にならない
- 子を予約するとき、親の sid ディレクトリがなければ失敗する（MUST）
- 予約の直後に `.sids/<sid>/started` を空で書く（MUST）。この mtime が `started_at`。sid ディレクトリ自身の mtime はロックや meta の書き込みで動くので使わない
- entry を作れなかったときは `.sids/<sid>/` ごと消して予約を取り消す（SHOULD）。残すと meta が以後のイベントを吸い込む
- sid の一意性と親子関係は 1 つの board の中でだけ保証される

## 5. entry のファイル名

```
{status}__{sid}__{agent}__{name}[ {message}]
```

```
^(10-waiting|20-working|30-background|40-idle|90-ended)
__([0-9a-z]{4}(\.[0-9a-z]{2})*)
__([a-z0-9-]+)
__([a-z0-9-]+)( (.+))?$
```

| 項目 | 規則 |
|---|---|
| 区切り | 項目の間は `__`。message の前だけ半角スペース 1 つ（MUST） |
| `status` | §3 の 5 つ（MUST） |
| `sid` | §4 |
| `agent` | エージェントの種類。`claude` など。`[a-z0-9-]+`（MUST） |
| `name` | 呼び名。`[a-z0-9-]+`（MUST）。64 文字以内（MUST。message を空にしてもファイル名が 255 バイトに収まる長さ） |
| `message` | 任意。§6 で整えた文字列。空なら区切りのスペースも省く |
| 拡張子 | なし |

- **どの項目の値にも `_` を入れない**（MUST）。`_` は区切り専用。`agent` と `name` は文字集合で自動的に守られ、message は §6 の整形で `_` を空白にする
- **解析の手順**: 先頭から `__` で 3 回だけ分ける（4 つ目の部分に `__` が残っていてもそれ以上は分けない）。4 つ目を最初のスペースで `name` と `message` に分ける。message の中のスペースはそのまま残る
- ファイル名全体が 255 バイトを超えるなら、収まるまで message の末尾をコードポイント単位で削り、そのたびに §6 の手順 3 を当て直す（MUST）。message を空にしても超えることは、name の上限があるので起きない
- 自分の entry は `*__{sid}__*` で見つかる。sid の後ろに必ず `__` が続くので、子の entry（`{sid}.01`）には一致しない
- 同じ sid の entry が複数あれば mtime が一番新しいものを使い、他は消さない（MUST。lint が報告する）

```console
$ ls board/
10-waiting__k3f9.01__claude__explorer approve Bash
20-working__k3f9__claude__conductor
40-idle__m7q2__claude__worker
```

### 任意の文字列を agent / name に寄せる

`lsboard run --name` やサブエージェントの種類名など、外から来た文字列は次のように token にする。

1. 小文字にする
2. `[a-z0-9]` 以外の文字の連続を 1 つの `-` にする
3. 前後の `-` を除く。空になれば不正（`run --name` はエラー、サブエージェントの種類名は `subagent`）
4. name は 64 コードポイントに切り詰め、末尾の `-` を除く。空になれば `x`

## 6. message の整形

message は人が見るべき理由を書く欄で、`10-waiting` のときに書く（SHOULD）。writer は書く前に次の順で整える（MUST）。

1. 次の文字を空白（U+0020）に置き換える: 制御文字（Unicode の一般カテゴリ Cc）、書式文字（Cf。ゼロ幅文字、文字の向きを変える U+202E など）、すべての空白文字（タブ、改行、NBSP、全角スペースを含む）、`/ \ < > : " | ? * _`
2. 連続する空白を 1 つにし、末尾の `.` をすべて除き、前後の空白を除く
3. コードポイント単位で 30 文字に切り詰め、もう一度 2 を行う

Unicode NFC への正規化は行わない。

| 入力 | 結果 |
|---|---|
| `approve Bash` | `approve Bash` |
| `承認待ち: design r4⏎` | `承認待ち design r4` |
| `run tests/unit⇥then gate...` | `run tests unit then gate` |
| `snake_case__name` | `snake case name` |
| `pick a⟨U+202E⟩redaeh` | `pick a redaeh` |
| `この文字列は三十文字を超えるのでコードポイント単位で切り詰められる` | `この文字列は三十文字を超えるのでコードポイント単位で切り詰め` |
| `abcdefghijklmnopqrstuvwxyz012 ...tail` | `abcdefghijklmnopqrstuvwxyz012` |
| `   `（空白のみ） | （空） |

⏎ は改行、⇥ はタブ、⟨U+202E⟩ は文字の向きを変える書式文字。

message は切り詰められるので、**承認そのものは board ではなくエージェントの画面で、内容を確かめてから行う**。reader は message をデータとして扱い、指示として解釈しない（MUST）。ファイル名を `eval` や `sh -c` に渡さない（MUST）。

## 7. meta（`.sids/<sid>/` のファイル）

ファイル名に出さない付随情報。1 ファイル 1 値で、末尾の改行は無視して読む。

| ファイル | 内容 | 誰がいつ書くか |
|---|---|---|
| `started` | 空。mtime が `started_at` | 予約の直後に 1 回（MUST） |
| `agent` | entry の `agent` と同じ値 | `run` が起動時に。サブエージェントは親の値を写す |
| `name` | entry の `name` と同じ値 | `run` が起動時に。サブエージェントは種類名 |
| `pid` | `lsboard run` 自身のプロセス ID | `run` が起動時に。サブエージェントは親の値を写す。lint の GONE 判定に使う |
| `session` | エージェントのセッション ID | hook（Claude Code は SessionStart） |
| `transcript` | transcript のパス | hook（Claude Code は SessionStart） |
| `waiting` | 待ちの出どころ。`agent` か `hook <64 桁の hex>` | `set waiting` が `agent` を、承認要求の hook が `hook <ハッシュ>` を書く。待ちを抜けるとき消す（MUST） |
| `agent_id` | サブエージェントの ID | 親セッションの hook が SubagentStart で。この meta を持つ sid は「hook が作った子」 |
| `settings.json` | Claude Code に渡す hook の設定 | `run` が起動時に書き、終了時に消す |
| `lock/` | 更新ロック（ディレクトリ） | §8 |

独自の meta を足してもよい。名前は `[a-z_]+` にする（SHOULD）。

## 8. ロック

- 更新と終了の前に `mkdir board/.sids/<sid>/lock` でロックを取る（MUST）。取れなければ 100 ミリ秒おきに再試行し、期限を過ぎたら諦める。期限は CLI（`set` `end` `run` の後始末）で 2 秒、hook で 0.5 秒
- 諦めたときは何も書かずに失敗する（MUST）。hook はエージェントを止めないので、その更新は失われるが、次のきっかけで正しい状態に戻る
- 処理が終わったら `rmdir` で外す
- **古いロックを自動で消さない**（MUST）。lint が 60 秒より古いロックを報告し、人が消す
- `touch`（§9）はロックを取らない

## 9. 操作の手順

**作成**（`run` と SubagentStart）

1. sid を予約し、`started` と meta を書く
2. `status` を `40-idle`（`run`）または `20-working`（サブエージェント）にしてファイル名を組み立て、**すでにあれば失敗する方法**で空ファイルを作る（`O_CREAT|O_EXCL`。MUST）
3. 失敗したら予約を取り消す（SHOULD）

**更新**

1. ロックを取る（§8）
2. 自分の entry を探す。**見つからなければ、何もせずにロックを外して終わる**（MUST）。管理者が `.ended/` に移した entry を勝手に戻さないため
3. 新しい値からファイル名を組み立て、名前が変わるなら今の名前から新しい名前へ rename する（MUST）
4. entry の mtime を今にする
5. ロックを外す

rename は原子的なので、どの時点でも entry は 1 つしかない。

**生存の印（touch）**: 状態が変わらないときは mtime だけを今にする。名前は変えない。ファイルが消えていれば（rename の直後など）成功として扱う（MUST）。

**終了（end）**

1. ロックを取る
2. 自分の entry を探す。なければ何もしない
3. `status` を `90-ended` にし、message を空にして、`.ended/` の中へ rename する（MUST）
4. mtime を今にし、`waiting` の meta を消す
5. ロックを外す

**家族の終了（end_tree）**: 自分の子孫を深いものから順に終わらせ、最後に自分を終わらせる。子孫の失敗は報告して続け、自分の失敗はエラーとして返す（MUST）。`lsboard end SID` と `run` の後始末、hook の SessionEnd が使う。

## 10. 時刻の意味

| 時刻 | どこにあるか |
|---|---|
| `started_at` | `.sids/<sid>/started` の mtime。予約の時刻で、以後動かない |
| `updated_at` | entry の mtime。最後に状態が変わったか、生存の印を付けた時刻 |

`.ended/` に移った entry の mtime は終了の時刻。

**止まっているかの判定**はファイルに書かず、reader が導く。`20-working` のまま mtime が古い（STALE）、`pid` のプロセスがない（GONE）など。GONE の判定は `pid` と同じ PID 名前空間からしかできない。

## 11. wait と watch

どちらも board のディレクトリと `.ended/` を見張り、2 回のスナップショットの差から変化を取り出す。ファイルシステムの通知（macOS は FSEvents、Linux は inotify）を使い、保険として 2 秒おきにも読み直す。通知が使えない環境では 2 秒のポーリングだけで動く。

**対象（scope）**: `--children-of SID` で指定した sid の**直接の子**だけ。孫は含まない。既定は `LSBOARD_SID`。`wait` では必須、`watch` では省略すると全員が対象。

**届ける変化**（`--until` の既定は `waiting,idle,ended`）

| 変化 | 条件 |
|---|---|
| 現れた | 前回のスナップショットになかった sid が scope の中にある。status は問わない |
| 変わった | 同じ sid の status が変わり、新しい status が `--until` に含まれる |

`20-working` と `30-background` への遷移は届けない。`.ended/` へ移った entry は `90-ended` への遷移として届く。

**まとめ（coalesce）**: 最初の変化から 1 秒の時間窓に起きた変化を 1 回の出力にまとめる。同じ sid の変化は最新の状態の 1 行だけにする。窓の最初が「現れた」ならその種別を保つ（MUST）。

**出力**: 1 行 1 entry、sid 順。

```
<sid> <status> <name>[ <message>]
k3f9.01 10-waiting explorer approve Bash
```

`--json` では 1 行 1 JSON。`event` は `"appeared"`、`"changed"`、または初期判定で出したときの `null`。

```json
{"event":"changed","sid":"k3f9.01","status":"10-waiting","agent":"claude","name":"explorer","message":"approve Bash"}
```

**wait の判定**

- 開始時にまず**生きている entry だけ**を見る（MUST）。`.ended/` の古い entry で即座に戻らないため。すでに条件を満たす子がいれば、それらを出して終了コード 0 で戻る
- `--any`（既定）: scope の中のどれかが `--until` の status になったら戻る。判定は、溜めた変化ではなく最後のスナップショットで行う
- `--all`: 開始時に生きていた子の全員が `--until` の status になるまで待つ。開始後に現れた子は数えない。スナップショットから消えた子は、`--until` に `ended` があれば終わったとみなす。開始時に生きている子が 1 人もいなければ、stderr に `no live children of <sid>` と出して終了コード 2 で戻る
- `--timeout SEC`: 過ぎたら、溜まっている変化を出してから終了コード 2 で戻る。出した変化が条件を満たしていれば 0。負の値は 0 秒、`inf` は期限なし

| 終了コード | 意味 |
|---|---|
| 0 | 条件を満たした |
| 1 | エラー（board がない、scope がない、sid が不正） |
| 2 | timeout、または `--all` で対象の子がいない |

**watch** は止められるまで流し続ける。stdout が閉じられたら終了コード 0 で終わる。

## 12. ls の出力

`lsboard ls` は生きている entry を整形して出す。生の `ls` でも読めるので、これは補助である。

- 既定の並びは status、sid の順。`--tree` では sid 順にし、深さ 2 文字ずつ字下げし、生きている子がいる行の末尾に `[N live child]`（複数なら `children`）を添える
- 列は status、sid、agent、name、message の順で、幅をそろえる
- `--json` は配列で、各要素は `sid` `status` `agent` `name` `message` `children`（生きている直接の子の数）`started_at` `updated_at`（Unix 秒。取れなければ null）`age_secs`（`updated_at` からの経過秒）

## 13. hook の共通規則

- `LSBOARD_SID` と `LSBOARD_DIR` の両方がなければ何もしない（MUST）。常設で hook を登録しても、`lsboard run` を通さない起動では何も起きない
- 標準入力が空なら何もしない
- **常に終了コード 0 で終わる**（MUST）。エラーは stderr に出す
- 1 回の実行は短く終わる（SHOULD。参考実装は数ミリ秒）。ロックの待ちは 0.5 秒まで
- 待ちの規則: エージェントが宣言した待ち（`waiting` が `agent`）を hook は上書きも解除もしない（MUST）。解除するのは人のプロンプト送信だけ。待ちを抜けるときは message と `waiting` の meta を消す（MUST）。迷ったら解除しない（SHOULD）

エージェントごとの遷移表は [hooks/](hooks/) に置く。

## 14. lint の報告

ファイルシステムは何でも受け付けるので、規約は書くときの検証と `lsboard lint` で守る。lint は次を報告し、1 件でもあれば終了コード 1 で終わる。

- 解析できないファイル名（`.` で始まらないのに §5 に合わない）
- 同じ sid の entry が複数ある
- 予約（`.sids/<sid>/`）のない entry
- 子が生きているのに親が生きていない
- `pid` のプロセスがいない（同じ PID 名前空間から見たとき）
- 60 秒より古いロック
- entry が board にも `.ended/` にもないのに、60 秒より古い予約が残っている

## 15. 片付け

- 正常に終わったら writer が entry を `.ended/` に移す
- 異常終了で残った entry は、管理者が lint の報告を見て `lsboard end SID` で移す
- `.sids/` の予約は、entry を `.ended/` から消すまで残す（sid の再利用を防ぐため）。`.ended/` をいつまで残すかは運用で決める
