# docrev

A terminal document viewer with inline review comments, designed for AI agent workflows.

Open a document in your terminal, leave comments anchored to its content, and let an
AI agent read them through a CLI, act on them, and reply — think code review, but for
documents.

> **Status: early development.**
> v0.1 targets Excel (`.xlsx`), read-only. Word (`.docx`) support is planned.
> Progress is tracked in the [v0.1.0 milestone](https://github.com/kaneko1117/docrev/milestone/1).

## Usage

```text
docrev file.xlsx        # browse the workbook in a TUI
docrev dump file.xlsx   # print a sheet as a text table (--sheet <name> to pick one)
```

### Viewer keys

| Key | Action |
|-----|--------|
| Arrow keys | Move the cursor |
| PgUp / PgDn | Page up / down |
| Home / End | First / last column of the row |
| Ctrl+Home / Ctrl+End | Top / bottom of the sheet |
| Tab / Shift+Tab | Next / previous sheet |
| c | Comment on the cell (replies when the cell already has an open thread) |
| r | Reply to the thread on the cell |
| q / Ctrl+C | Quit |

In the comment editor: Enter inserts a newline, Ctrl+S saves, Esc cancels.
The status bar shows the full value of the selected cell; cells with an open
thread are marked with `●` and their thread appears in a side panel.

## Planned

```text
docrev comment list file.xlsx --json  # an agent reads the comments you left in the TUI
docrev comment add file.xlsx ...      # ...and replies inline
```

Comments will live in a sidecar file (`file.xlsx.docrev.json`); the original document
is never modified.

## License

MIT OR Apache-2.0 (license files land with the first release).
