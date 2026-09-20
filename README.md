# docrev

[日本語版 README](README.ja.md)

A terminal document viewer with inline review comments, designed for AI agent workflows.

Open a document in your terminal, leave comments anchored to its content, and let an
AI agent read them through a CLI, act on them, and reply — think code review, but for
documents. A Markdown file is shown line by line with line numbers; a workbook gets a
spreadsheet-style grid (white canvas, gridlines, formula bar) — both right in your
terminal.

![One comment on a Markdown heading asks Claude to check schedule.xlsx; Claude fixes the lines and the viewer shows the edit and the reply on its own](demo/demo-markdown.gif)

> Markdown (`.md`) and Excel (`.xlsx`), read-only. Word (`.docx`) support is planned.

## Installation

```text
# macOS / Linux
brew install kaneko1117/tap/docrev
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/kaneko1117/docrev/releases/latest/download/docrev-installer.sh | sh

# Windows
powershell -c "irm https://github.com/kaneko1117/docrev/releases/latest/download/docrev-installer.ps1 | iex"

# with a Rust toolchain
cargo install docrev
```

Prebuilt binaries for macOS, Linux and Windows are attached to every
[release](https://github.com/kaneko1117/docrev/releases).

## Usage

```text
docrev notes.md                    # read a Markdown file in a TUI
docrev file.xlsx                   # browse the workbook in a TUI
docrev dump notes.md               # print the file with line numbers, like cat -n
docrev dump file.xlsx              # print a sheet as a text table (--sheet <name> to pick one)
docrev dump file.xlsx --formulas   # formulas instead of results, like Excel's Ctrl+`
```

### Markdown

A Markdown file opens as its source lines, numbered the way `cat -n` numbers
them. Headings, emphasis, inline code, lists, tables and fenced code are styled
in place, but one line of the file always stays one line on screen (a long
line wraps under a single number), so the numbers you see are the ones an
agent uses. The side panel is always open (when the terminal is wide enough)
and shows the cursor line's thread.

| Key | Action |
|-----|--------|
| ↑ / ↓ | Move the cursor line |
| PgUp / PgDn | Page up / down |
| Home / End, Ctrl+Home / Ctrl+End | First / last line |
| Ctrl+F | Find in the file (type to jump, ↓/↑ next/previous, Enter to stay, Esc to go back) |
| c | Comment on the line (continues the line's thread when it has one, reopening a resolved one) |
| q / Ctrl+C | Quit |

Click a line to select it; the wheel scrolls. **Drag across lines to copy
them** as written in the file, markup included. Lines with an open thread are
marked with `●`.

### Excel

![Comment on a cell in the viewer, Claude picks it up over the CLI, and the reply lands back in the viewer on its own](demo/demo.gif)

| Key | Action |
|-----|--------|
| Arrow keys | Move the cursor |
| PgUp / PgDn | Page up / down |
| Home / End | First / last column of the row |
| Ctrl+Home / Ctrl+End | Top / bottom of the sheet |
| Tab / Shift+Tab | Next / previous sheet |
| Ctrl+G / F5 | Go to a sheet by name (type to filter, Enter to switch) |
| Ctrl+F | Find on the active sheet (type to jump, ↓/↑ next/previous, Enter to stay, Esc to go back) |
| c | Comment on the cell (continues the cell's thread when it has one, reopening a resolved one) |
| n | View the workbook's own Excel comments on the cell (read-only; Esc closes) |
| q / Ctrl+C | Quit |

Click a cell to select it, a sheet tab to switch, `‹`/`›` to step through
sheets; the wheel scrolls (Shift+wheel sideways). **Drag across cells to
copy the range** — the clipboard receives the full underlying values as
tab-separated text, ready to paste into Excel or Google Sheets as a table.
Copying uses OSC 52, so it reaches your local clipboard even over SSH. A
click on an open sheet picker, search or notes closes it and acts at once.

The formula bar shows the selected cell's formula (`=SUM(E7:E34)`) when it
has one, and its full value otherwise — the grid keeps showing results,
like Excel. Cells with an open
thread are marked with `●`; press `c` on one to open its thread in a side
panel — read and `Esc` out, or type and `Ctrl+S` to reply. (On terminals too
narrow for the panel, `c` still opens the reply editor.) A cell holds one
thread: `c` always continues it, and a reply on a resolved thread reopens it.
Moving the cursor alone never opens the panel, so the grid keeps its width.
Cells carrying the
workbook's own Excel comments show a tinted top-right corner; press `n` to
read them. Frozen
panes saved in the workbook are honored: pinned rows and columns stay on
screen while the rest scrolls.

### Comment editor

`c` opens the editor on the cursor's line or cell. Enter inserts a newline,
Ctrl+S saves, Esc closes it. The cursor keys and clicks (sheet tabs included)
keep working while you type: the editor follows the cursor, and each line or
cell keeps its own unsaved draft — also after Esc — until you save it or quit
docrev. Drafts are never written to disk.

### Colors

The viewer paints a spreadsheet-style white canvas by default. To keep your
terminal's own palette instead:

```text
docrev file.xlsx --theme terminal
export DOCREV_THEME=terminal        # or set it once
```

`--theme` wins over `DOCREV_THEME`. Workbook fill and font colors are absolute
RGB meant for white paper, so the `terminal` theme leaves them out.

## Agent CLI

The other half of the loop — an AI agent reads your comments, acts, and replies:

```text
docrev comment list <file> --json [--unresolved] [--author <name>] [--sheet <name>]
docrev comment add notes.md --line 13 --body "..." [--author <name>]
docrev comment add file.xlsx --cell "Sheet1!B3" --body "..." [--author <name>]
docrev comment reply <file> --thread <id> --body "..." [--author <name>]
docrev comment resolve <file> --thread <id>
```

`list --json` emits the [sidecar schema](docs/sidecar.md), with each thread
carrying what it is anchored to — on a Markdown file the line's text with the
two lines on each side, on a workbook the cell's content and its row — so a
batch of comments is actionable without reading the document. `--line` takes
the 1-based line number the viewer shows; a comment stays on its line number
when the file is edited, it does not follow the text. `--sheet` is for
workbooks.

`add`/`reply`/`resolve` print the affected thread (including its id); `add` on
a line or cell that already has a thread appends to it instead of starting a
second one. Comments live in a sidecar file (`notes.md.docrev.json`); the
original document is never modified, and concurrent TUI/CLI writes are
serialized through a `.lock` file.

## Using with Claude (or any agent)

[`skills/docrev-review/SKILL.md`](skills/docrev-review/SKILL.md) teaches an agent the
full loop. For Claude Code, install it as a plugin — the repository is its own
marketplace:

```text
/plugin marketplace add kaneko1117/docrev
/plugin install docrev@docrev
```

Any other agent can use the skill file directly; for Claude Code that means
copying it into your skills directory:

```text
mkdir -p ~/.claude/skills/docrev-review
cp skills/docrev-review/SKILL.md ~/.claude/skills/docrev-review/
```

Then comment on lines or cells in the viewer, tell Claude "I commented on
notes.md" (or "on budget.xlsx"), and watch the replies appear — the viewer
picks them up on its own.

## License

MIT OR Apache-2.0, at your option.
