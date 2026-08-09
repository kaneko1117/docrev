# CLAUDE.md

docrev — a terminal document viewer with inline review comments, designed for AI agent workflows.

A user opens a document in a TUI, leaves comments anchored to locations in it (cells, later paragraphs), and an AI agent reads those comments through a CLI, acts on them, and replies. Think "hunk, but for documents instead of diffs".

v0.1 scope: Excel (`.xlsx`) only, read-only viewer, comments stored in a sidecar JSON file, agent-facing CLI. Word (`.docx`) support is planned — never let that door close.

## Architecture

Clean architecture. Layers and dependency rules:

```
src/
  main.rs      Composition root. The ONLY place that knows every layer:
               builds domain objects, wires adapters to ports, runs the app.
  domain/      Pure model: Document, Sheet, Cell, Comment, Anchor, ...
               No IO, no dependencies on any other layer, no external crates
               beyond std (serde derive is the one allowed exception).
  app/         Use cases (open a review, add a comment, list comments, ...).
               Depends on domain only. Everything it needs from the outside
               world is declared HERE as a trait (a "port"), e.g.
               `DocumentSource`, `CommentStore`, `Screen`.
  adapter/     Implements app's ports by translating them to infra/ui.
               This is the only layer that maps external formats to domain types.
  infra/       Real-world primitives: file IO, xlsx parsing (calamine),
               sidecar JSON persistence, terminal backend. Knows nothing
               about app; may be used only via adapters.
  ui/          ratatui rendering: grid, comment panel, popups, status bar.
               May read domain types to draw them; never mutates state.
```

- Dependencies point inward: `ui/infra → adapter → app → domain`. Never the reverse.
- Format-specific types (calamine's, serde_json's) must not leak above `adapter`.
- Unit tests in `app` use fake port implementations (no real files, no real terminal).

## Domain decisions

- `Anchor` is an enum from day one:
  `Anchor::Cell { sheet: String, row: u32, col: u32 }` now;
  a paragraph/range variant will be added for Word later. Everything that
  touches comments goes through `Anchor`, never through raw cell strings.
- Coordinates are **0-based** everywhere internally. A1-notation ("Sheet1!B12")
  exists only at the edges: CLI arguments, JSON sidecar, and UI display.
  Conversion lives in one place in `adapter`.
- The sidecar file is `<document>.docrev.json`, versioned (`"version": 1`).
  Its schema is a public contract (agents depend on it) — document every change.

## Coding rules

- No `unwrap`/`expect` outside tests. Errors are typed (`thiserror`) and
  propagate with `?`; `main` decides how to present them.
- `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must pass before every PR.
- Everything in this repo is in English: code, comments, doc comments, CLI help,
  error messages, commit messages, README.
- Conversation with the user may be in Japanese; artifacts are English.

## Workflow

- Trunk-based: `main` is the trunk, one feature branch per stage, PR into `main`.
  No `develop` branch. Releases are git tags (`v0.1.0`) published to crates.io.
- Never push directly to `main` — always via PR. Branch protection (PR required,
  1 approval, admin bypass allowed) will be enforced when the repo goes public;
  until then the same rule applies by convention.
- Keep every PR small — under 1000 lines including tests, ideally far less.
- The user reviews diffs in a live Hunk session and leaves inline comments there;
  fetch them with `hunk session comment list --type user` when the user says they
  commented, and reply inline via `hunk session comment apply`.
- An adversarial review pass runs before merge (user-invoked skill). Expect it.
- Do not run `cargo init`, `git init`, or create repositories — the user does
  scaffolding themselves.

## Roadmap

Progress is tracked in **GitHub Issues** (milestone `v0.1.0`) — Issues are the source
of truth; the table below is a static summary. Check current state with
`gh issue list --milestone v0.1.0`. Each stage PR closes its issue via `Closes #N`.

| Stage | Issue | Deliverable |
|-------|-------|-------------|
| 1 | #1 | CLI skeleton + `docrev dump file.xlsx` (calamine parsing proven) |
| 2 | #2 | Read-only TUI: grid, cursor, sheet switching, status bar |
| 3 | #3 | Comment input in TUI + sidecar JSON store |
| 4 | #4 | Agent CLI: `docrev comment list/add/resolve --json` |
| 5 | #5 | Agent skill (SKILL.md), README polish, LICENSE, v0.1.0 to crates.io |

Post-v0.1 backlog (no milestone): #6 live reload, #7 native xlsx threaded-comment
write-back, #8 `.docx` support.
