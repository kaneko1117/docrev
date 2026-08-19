# docrev

[English README](README.md)

ターミナルで動くドキュメントビューア + インラインレビューコメント。AI エージェントとの協働を前提に設計されています。

ドキュメントをターミナルで開き、セルにコメントを付けると、AI エージェント(Claude など)が CLI 経由でそれを読み、対応し、返信します — 「コードレビューのドキュメント版」です。ビューアはスプレッドシート風のグリッド(白いキャンバス・グリッド線・数式バー)をターミナル内に描画します。

> Excel(`.xlsx`)の読み取り専用に対応。Word(`.docx`)対応は計画中です。

## インストール

```text
# macOS / Linux
brew install kaneko1117/tap/docrev
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/kaneko1117/docrev/releases/latest/download/docrev-installer.sh | sh

# Windows
powershell -c "irm https://github.com/kaneko1117/docrev/releases/latest/download/docrev-installer.ps1 | iex"

# Rust の環境がある場合
cargo install docrev
```

macOS・Linux・Windows 向けのビルド済みバイナリは、各[リリース](https://github.com/kaneko1117/docrev/releases)に添付されています。

## 使い方

```text
docrev file.xlsx        # ビューアでブックを開く
docrev dump file.xlsx   # シートをテキスト表として出力(--sheet <名前> で選択)
```

### ビューアのキー操作

| キー | 動作 |
|------|------|
| 矢印キー | カーソル移動 |
| PgUp / PgDn | ページ送り |
| Home / End | 行の先頭 / 末尾 |
| Ctrl+Home / Ctrl+End | シートの先頭 / 末尾 |
| Tab / Shift+Tab | シート切替 |
| Ctrl+G / F5 | シート名で移動(文字で絞り込み、Enter で切替) |
| Ctrl+F | シート内検索(文字で移動、↓↑で次/前、Enter でその場に留まる、Esc で元の場所へ) |
| c | セルにコメント(既にスレッドがあるセルでは返信) |
| r | スレッドに返信 |
| q / Ctrl+C | 終了 |

### マウス操作

セルをクリックで選択、下のタブをクリックでシート切替、`‹`/`›` で隣のシートへ。ホイールで上下(Shift+ホイールで左右)に動きます。**セル範囲をドラッグすると、その範囲がコピーされます** — 画面の見た目ではなくセルの完全な値がタブ区切りで入り、Excel や Google スプレッドシートにそのまま表として貼り付けられます。コピーは端末経由(OSC 52)なので SSH 越しでも手元のクリップボードに届きます。何かが開いているときも、クリックが常に優先されます。

コメントエディタ内: Enter = 改行、Ctrl+S = 保存、Esc = キャンセル。
数式バーには選択中のセルの値全体が表示されます。未解決スレッドのあるセルには `●` マーカーが付きます。`c` を押すとスレッドが右パネルに開き、読むだけなら Esc、返信するならそのまま入力して Ctrl+S(パネルが入らない狭い画面では返信欄だけが開きます)。カーソルを合わせただけではパネルは開かず、表の幅は変わりません。Excel で設定したウィンドウ枠の固定はそのまま反映され、見出しの行・列が画面に残ります。

### 配色

既定ではスプレッドシート風の白い画面で表示します。ターミナル自身の配色を使いたい場合:

```text
docrev file.xlsx --theme terminal
export DOCREV_THEME=terminal        # 毎回指定したくない場合
```

`--theme` は `DOCREV_THEME` より優先されます。なお Excel ファイルが持つ塗りつぶし色と文字色は白い背景を前提にした色なので、`terminal` では反映しません。

## エージェント向けコマンド

エージェント(Claude など)は、次のコマンドであなたのコメントを読み、対応し、返信します:

```text
docrev comment list file.xlsx --json [--unresolved] [--author <名前>] [--sheet <名前>]
docrev comment add file.xlsx --cell "Sheet1!B3" --body "..." [--author <名前>]
docrev comment reply file.xlsx --thread <id> --body "..." [--author <名前>]
docrev comment resolve file.xlsx --thread <id>
```

`list --json` の出力には、各コメントに**そのセルの中身と同じ行の内容**が同梱されます。エージェントはシートを読みに行かなくても、受け取ったコメントの束にそのまま着手できます。

コメントは元のファイルの隣に作られる専用ファイル(`file.xlsx.docrev.json`)に保存され、**元のドキュメントには一切書き込みません**。ビューアとエージェントが同時に書き込んでも壊れないよう保護されています。ファイル形式の詳細は [docs/sidecar.md](docs/sidecar.md) にあります。

## Claude と使う

[`skills/docrev-review/SKILL.md`](skills/docrev-review/SKILL.md) にエージェント向けの手順書があります。Claude Code なら skills ディレクトリにコピーしてください:

```text
mkdir -p ~/.claude/skills/docrev-review
cp skills/docrev-review/SKILL.md ~/.claude/skills/docrev-review/
```

あとはビューアでコメントを付けて Claude に「budget.xlsx にコメントした」と伝えるだけ。返信はビューアが自動で拾って表示します。

## ライセンス

MIT または Apache-2.0(選択可)。
