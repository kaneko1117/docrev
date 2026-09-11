# Sidecar file format

Comments live next to the document in `<document>.docrev.json`
(e.g. `budget.xlsx` → `budget.xlsx.docrev.json`). The document itself is never
modified. The sidecar is written atomically (temp file + rename), so readers
never observe a half-written file.

This format is a **public contract**: AI agents read and write it through the
`docrev comment` CLI. Breaking changes require a `version` bump and an entry here.

## Example

```json
{
  "version": 2,
  "comments": [
    {
      "id": "3e1f0b6c-6a86-4b8e-9f0e-2d8b3a4f5c6d",
      "anchor": { "sheet": "Sales", "cell": "B3" },
      "author": "user",
      "body": "Isn't this unit price outdated?",
      "created_at": "2026-08-11T09:15:00Z",
      "resolved": false,
      "replies": [
        {
          "id": "9c2d1e4f-5b6a-4c7d-8e9f-0a1b2c3d4e5f",
          "author": "claude",
          "body": "Checked — the current price is 150.",
          "created_at": "2026-08-11T09:20:00Z"
        }
      ]
    },
    {
      "id": "5a7c2d1e-0f3b-4a9c-8d2e-6b1f4c7a9e0d",
      "anchor": { "kind": "line", "line": 13 },
      "author": "user",
      "body": "brew tap comes first, doesn't it?",
      "created_at": "2026-09-11T01:02:00Z",
      "resolved": false,
      "replies": []
    }
  ]
}
```

## Fields

| Field | Type | Notes |
|-------|------|-------|
| `version` | int | Schema version. Currently `2`; readers accept `1` and `2` and reject anything else. Every write emits `2`, so a version-1 file becomes version 2 the first time docrev writes it |
| `comments` | array | Comment threads in creation order: writers append |
| `comments[].id` | string | UUIDv4, assigned by the writer |
| `comments[].anchor` | object | Where the thread sits; one of the shapes below |
| `comments[].anchor.sheet` | string | Cell anchor: sheet name |
| `comments[].anchor.cell` | string | Cell anchor: A1 notation (`"B3"`) |
| `comments[].anchor.kind` | string | Line anchor: always `"line"`. Absent on a cell anchor |
| `comments[].anchor.line` | int | Line anchor: 1-based physical line of the text file |
| `comments[].author` | string | `"user"` for the human reviewer; agents use their own name (e.g. `"claude"`) |
| `comments[].body` | string | Comment text, may contain newlines |
| `comments[].created_at` | string | ISO 8601 UTC (`2026-08-11T09:15:00Z`) |
| `comments[].resolved` | bool | Applies to the whole thread. Defaults to `false` when absent |
| `comments[].replies` | array | Replies in chronological order. Defaults to `[]` when absent |
| `replies[].id` / `author` / `body` / `created_at` | | Same semantics as the thread fields |

## Semantics

- A **thread** is one root comment plus its replies; `resolved` closes the whole
  thread. **Replying reopens it** (`resolved` returns to `false`): a reply on a
  closed thread would otherwise be invisible to the viewer, which only marks
  open threads, and to agents, which list unresolved ones.
- **One cell holds one thread.** Several threads on a cell are valid in the
  schema, but docrev never creates a second one: `c` in the viewer and
  `comment add` on the CLI both continue the cell's existing thread, resolved or
  not (a merged region counts as one cell, anchored at its top-left). When a
  file written by hand or by an older docrev does hold several, the cell's
  thread is the first unresolved one, else the last one in the file. A line
  of a text document holds one thread by the same rule.

## Anchor kinds

`anchor` has two shapes, told apart by the presence of `"kind"`:

- **Cell** — `{"sheet": "Sales", "cell": "B3"}`, never with a `kind` key. This
  is the only shape version 1 knew.
- **Line** — `{"kind": "line", "line": 13}`, for text documents (Markdown).
  `line` is the 1-based physical line of the file, the number `cat -n` shows.
  Like a cell anchor, it is stored as-is: editing the file does not move the
  comment, so a comment on line 13 stays on line 13 after lines are inserted
  above it.

A reader treats anything else as corrupt — a `kind` it does not know
(`"kind": "cell"` included), a line anchor without `line` or with `line: 0`,
an object with neither a cell nor a kind — and refuses to load the file.

Version 2 added the line kind; version 1 files load unchanged. The next kind
(a Word paragraph, say) gets its own `kind` value and a version bump; docrev is
one binary reading one file, so there is no staged compatibility beyond
"newer docrev reads older files".

## CLI

`docrev comment` is the intended way for agents to read and write this file:

- `list --json` prints this document shape (`{"version": 2, "comments": [...]}`)
  after applying filters — the schema above is the output contract, plus one
  **derived, output-only** addition: each cell thread carries a `cell` object with
  the anchored cell's displayed text and its row's other non-empty cells
  (`"cell": {"value": "...", "row": {"A2": "...", "D2": "..."}}`; `row` keys
  come in column order and the object may be empty). When a number format
  produced the anchored cell's display, `cell` also carries `"raw"` — the
  machine-readable value behind it. For a date or time cell it is a string:
  `"2026-08-31 00:00:00"` for date-bearing cells, `"13:05:00"` for time-only
  cells (never a fictional epoch date), and elapsed `"36:00:00"` for
  `[h]`-style durations; the rare cell an xlsx stores as ISO 8601 text
  (`t="d"`) passes that string through verbatim
  (`"2026-08-31T13:05:00"`-shaped). For a formatted number cell it is a JSON
  number, the value the workbook stores: `{"value": "1,234千円", "raw":
  1234000}`, `{"value": "15%", "raw": 0.15}`; an integral value within the
  64-bit integer range has no fractional part. `raw` is absent on every
  other cell kind (plain numbers and text already show their raw rendering
  in `value`) and on a formatted number that is not finite, so its JSON type
  tells dates from numbers and it is never `null`. It is computed from the
  workbook at list time and is **never stored in the sidecar**; writers must
  ignore a `cell` key on input. When the workbook cannot be read (corrupt
  file) or the sheet was renamed, `cell` is omitted for the affected threads
  and the command still succeeds; a document path that does not exist at all
  is still an error, as for every `comment` command. A line thread never
  carries `cell`, and `--sheet` matches cell anchors only, so it leaves line
  threads out. A merged anchor's
  `value` is its region's value, and the region's cells never repeat in
  `row`. `row` shows what a person sees: columns the workbook hides are
  left out, and a hidden row or a hidden sheet has an empty `row`. A thread
  whose anchor the workbook hides (a hidden row, column or sheet) carries a
  top-level `"hidden": true`; the key is absent otherwise, and absent when
  `cell` is, so its presence identifies threads a person cannot reach in the
  viewer. A merged region counts as shown while any of it is.
  The anchored cell's own `value` is always present. Never redirect this
  output onto the sidecar itself — the shell truncates the file before the
  command reads it.
- `list --json` also carries a second, **read-only** top-level array,
  `workbook_comments`: the workbook's own Excel comments (legacy notes and
  threaded comments), each as `{"anchor": {"sheet", "cell"}, "author",
  "body", "resolved", "replies": [{"author", "body"}]}`. They have no `id`
  and can never be replied to or resolved through docrev — which is exactly
  why they are kept out of `comments`. The `--sheet`, `--author` and
  `--unresolved` filters apply to them the same way. Derived from the
  workbook at list time; never stored in the sidecar.
- `add` / `reply` / `resolve` print the affected thread (same thread shape,
  including its `id`) and exit non-zero with a message on stderr for invalid
  cell references, unknown sheets, or unknown thread ids.
- Writers hold an exclusive advisory lock on `<sidecar>.lock` during
  read-modify-write, so concurrent TUI and CLI writes cannot lose updates.
  The lock file is left in place; it is safe to delete when nothing is running.
- Threads whose `resolved` is `false` are shown with a `●` marker in the viewer.
- A missing sidecar file means "no comments" and is not an error. A corrupt or
  unsupported sidecar must not prevent opening the document read-only.
