# docrev

A terminal document viewer with inline review comments, designed for AI agent workflows.

Open a document in your terminal, leave comments anchored to its content, and let an
AI agent read them through a CLI, act on them, and reply — think code review, but for
documents.

> **Status: early development — not usable yet.**
> v0.1 targets Excel (`.xlsx`), read-only. Word (`.docx`) support is planned.
> Progress is tracked in the [v0.1.0 milestone](https://github.com/kaneko1117/docrev/milestone/1).

## Planned v0.1 workflow

```text
docrev file.xlsx                      # browse sheets in a TUI, leave comments on cells
docrev comment list file.xlsx --json  # an agent reads your comments
docrev comment add file.xlsx ...      # ...and replies inline
```

Comments live in a sidecar file (`file.xlsx.docrev.json`); the original document is
never modified.

## License

MIT OR Apache-2.0 (license files land with the first release).
