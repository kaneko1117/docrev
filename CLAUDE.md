# CLAUDE.md

docrev — a terminal document viewer with inline review comments, designed for AI agent workflows.

A user opens a document in a TUI, leaves comments anchored to locations in it (cells, later paragraphs), and an AI agent reads those comments through a CLI, acts on them, and replies. Think "hunk, but for documents instead of diffs".

Scope: Excel (`.xlsx`) only, read-only viewer, comments stored in a sidecar JSON file, agent-facing CLI. Word (`.docx`) support is planned — never let that door close. (No version numbers or progress snapshots in docs — that state lives in GitHub issues and milestones, see Roadmap.)

## Architecture

Clean architecture. Layers and dependency rules:

```
src/
  main.rs      Composition root. The ONLY place that knows every layer:
               builds domain objects, wires adapters to ports, runs the app.
  domain/      docrev's own model of a reviewable document: Document, Sheet,
               Cell, Comment, Anchor, ... No IO, no dependencies on any other
               layer, no external crates beyond std (serde derive is the one
               allowed exception).
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
  Conversion lives in one place: the methods on `domain::anchor::Anchor`
  (`cell_ref`, `parse_cell_ref`, `column_label`).
- What belongs in `domain` is decided by subject matter, never by purity, and
  the line is **whose contract a thing is**. A1 notation is docrev's own — the
  sidecar and the CLI speak it — so `Anchor` owns it (see the bullet above).
  A workbook's number-format grammar is not: `#,##0;[Red]` is a rule docrev only
  ever reads out of someone else's file and never emits, so the engine that
  interprets it lives in `infra` however pure it is. `Sheet` and `CellValue`
  stay, because `domain` holds the model each `Anchor` points into. The test:
  swap xlsx for docx, the terminal for a web UI, the sidecar for a database —
  whatever must change was never `domain`. Purity is a consequence of belonging
  there, not a reason to be there.
- No constructor-less utility modules in `domain` — free functions belong next to
  their single consumer until a real domain type can own them.
- The sidecar file is `<document>.docrev.json`, versioned (`"version": 1`).
  Its schema is a public contract (agents depend on it) — document every change.

## Coding rules

- No `unwrap`/`expect` outside tests. Errors are typed (`thiserror`) and
  propagate with `?`; `main` decides how to present them.
- `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must pass before every PR.
- Everything in this repo is in English: code, comments, doc comments, CLI help,
  error messages, commit messages, README.
- Comments are the bare minimum, one sentence each. Write one only for: a
  contract the signature cannot show (`None`/error conditions, 0- or 1-based,
  units, call-order preconditions, side effects); the meaning of a tuple or
  primitive field (`(rows, cols)`); or the reason for code that looks wrong
  or unnecessary. Everything else — restating a name, design rationale,
  history, issue numbers, test narration — goes in the PR, not the code.
  Module docs (`//!`) are at most one line, and only for a layering
  constraint.
- Conversation with the user may be in Japanese; artifacts are English.

## Workflow

- Trunk-based: `main` is the trunk, one feature branch per stage, PR into `main`.
  No `develop` branch.
- Never push directly to `main` — always via PR. Branch protection (PR required,
  1 approval, admin bypass allowed) will be enforced when the repo goes public;
  until then the same rule applies by convention.
- Keep every PR small — under 1000 lines including tests, ideally far less.
- Commit messages follow **Conventional Commits** (`feat:`, `fix:`, `docs:`,
  `refactor:`, `perf:`, `test:`, `chore:`; `feat!:` for a breaking change).
  release-plz derives the version bump and the changelog from them, so a
  mislabelled commit ships the wrong version. `fix:` is only for defects a
  user could hit; internal cleanups, CI and tooling tweaks are `chore:`.
- The user reviews diffs in a live Hunk session and leaves inline comments there;
  fetch them with `hunk session comment list --type user` when the user says they
  commented, and reply inline via `hunk session comment apply`.
- After finishing an implementation, launch the `adversarial-reviewer` agent
  (`.claude/agents/adversarial-reviewer.md`) as a background subagent and fix
  what it confirms before asking for the merge.
- Do not run `cargo init`, `git init`, or create repositories — the user does
  scaffolding themselves.

## Releases

release-plz opens a release PR from the Conventional Commit history; merging
it tags the version, and the tag triggers cargo-dist, which builds the
platform binaries and installers and pushes the Homebrew formula to
`kaneko1117/homebrew-tap`. Notes learned the hard way:

- The tag must be pushed with credentials that can trigger downstream
  workflows (a PAT), or the dist jobs never start.
- The tap repository must contain at least one commit; publishing a formula
  into a completely empty repository fails with
  "couldn't find remote ref refs/heads/main".

## Roadmap

Progress lives in **GitHub Issues and milestones** — the source of truth,
never mirrored into docs. Check state with `gh issue list --milestone <name>`
(`gh api repos/{owner}/{repo}/milestones` lists the names). Each PR closes
its issue via `Closes #N`.
