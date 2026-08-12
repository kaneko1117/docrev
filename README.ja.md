# docrev

[English README](README.md)

ターミナルで動くドキュメントビューア + インラインレビューコメント。AI エージェントとの協働を前提に設計されています。

ドキュメントをターミナルで開き、セルにコメントを付けると、AI エージェント(Claude など)が CLI 経由でそれを読み、対応し、返信します — 「コードレビューのドキュメント版」です。ビューアはスプレッドシート風のグリッド(白いキャンバス・グリッド線・数式バー)をターミナル内に描画します。

> v0.1 は Excel(`.xlsx`)の読み取り専用に対応。Word(`.docx`)対応は計画中です。

## インストール

```text
cargo install docrev
```

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
| c | セルにコメント(既にスレッドがあるセルでは返信) |
| r | スレッドに返信 |
| F5 | コメント再読込(エージェントの返信を反映) |
| q / Ctrl+C | 終了 |

コメントエディタ内: Enter = 改行、Ctrl+S = 保存、Esc = キャンセル。
未解決スレッドのあるセルには `●` マーカーが付き、カーソルを合わせると右パネルにスレッドが表示されます。

## エージェント向けコマンド

エージェント(Claude など)は、次のコマンドであなたのコメントを読み、対応し、返信します:

```text
docrev comment list file.xlsx --json [--unresolved] [--author <名前>] [--sheet <名前>]
docrev comment add file.xlsx --cell "Sheet1!B3" --body "..." [--author <名前>]
docrev comment reply file.xlsx --thread <id> --body "..." [--author <名前>]
docrev comment resolve file.xlsx --thread <id>
```

コメントは元のファイルの隣に作られる専用ファイル(`file.xlsx.docrev.json`)に保存され、**元のドキュメントには一切書き込みません**。ビューアとエージェントが同時に書き込んでも壊れないよう保護されています。ファイル形式の詳細は [docs/sidecar.md](docs/sidecar.md) にあります。

## Claude と使う

[`skills/docrev-review/SKILL.md`](skills/docrev-review/SKILL.md) にエージェント向けの手順書があります。Claude Code なら skills ディレクトリにコピーしてください:

```text
mkdir -p ~/.claude/skills/docrev-review
cp skills/docrev-review/SKILL.md ~/.claude/skills/docrev-review/
```

あとはビューアでコメントを付けて Claude に「budget.xlsx にコメントした」と伝え、`F5` で返信を眺めるだけです。

## ライセンス

MIT または Apache-2.0(選択可)。
