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
  "version": 1,
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
    }
  ]
}
```

## Fields

| Field | Type | Notes |
|-------|------|-------|
| `version` | int | Schema version. Currently `1`; readers must reject unsupported values |
| `comments` | array | Comment threads, order not significant |
| `comments[].id` | string | UUIDv4, assigned by the writer |
| `comments[].anchor.sheet` | string | Sheet name |
| `comments[].anchor.cell` | string | A1 notation (`"B3"`) |
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
  open threads, and to agents, which list unresolved ones. Multiple threads per cell are valid in the schema, but the TUI follows
  the spreadsheet convention of one open thread per cell (`c` replies to an open
  thread instead of forking a second one).

## Anchor kinds and extension

Today `anchor` has exactly one shape — the cell anchor
`{"sheet": ..., "cell": ...}` — and that shape is frozen: agents parse it,
and it never gains or loses keys. Word support will need at least a
paragraph anchor, so the extension rule is fixed now, before any docx code
exists:

- **Every future anchor kind is a distinct object shape carrying a required
  `"kind"` discriminator** (for example `{"kind": "paragraph", ...}` — its
  fields are decided with the docx design). Cell anchors are grandfathered:
  the absence of `"kind"` means cell, and writers never emit a `"kind"` key
  on them. `"kind": "cell"` is reserved and must not be written — a reader
  treats it as an unknown kind, so a well-meaning writer that emits it
  makes its own comments invisible.
- **The first non-cell kind ships with a `version` bump to `2`**, and any
  file containing a non-cell anchor must declare version 2 or higher — the
  writer that first adds one raises the field. Version-2 readers keep
  accepting version 1 unchanged. One caveat keeps this honest: today's
  version-1 readers parse `comments` *before* checking `version`, so on a
  real v2 file they fail with an "invalid sidecar" corruption-shaped
  message and never reach the version check. Before any v2 writer ships, a
  v1.x release must move the version check ahead of comment parsing, so
  that old readers refuse with "unsupported sidecar version" — a message
  that says "upgrade", not "your file is broken". (Version `2` was once
  earmarked for an embedded `changes` array; that plan was retired with
  the issue that proposed it, so the number is free.)
- **From version 2 on, unknown anchor kinds are skipped and preserved.**
  A reader that meets a kind it does not know leaves the thread out of the
  TUI and out of `list`, does not let `reply`/`resolve` address it (same
  error as an unknown id), and — the binding part — **preserves it
  byte-faithfully when rewriting the file**. Read-modify-write must round-trip
  threads it cannot interpret (keep the raw JSON, don't re-serialize through
  typed structs). That makes `2` the last bump anchors ever force — provided
  shipped kinds are frozen: an incompatible change to an existing kind ships
  as a new kind name, never as a new shape under the old one, and a reader
  that knows a kind but cannot parse its fields treats the thread as
  unknown. Kind number three then never requires version `3`.

Checked against today's consumers: the agent skill drives everything through
the `docrev comment` CLI and only ever writes `Sheet!B3` cell references, so
nothing changes for it until a new kind actually ships.

## CLI

`docrev comment` is the intended way for agents to read and write this file:

- `list --json` prints this document shape (`{"version": 1, "comments": [...]}`)
  after applying filters — the schema above is the output contract, plus one
  **derived, output-only** addition: each thread carries a `cell` object with
  the anchored cell's displayed text and its row's other non-empty cells
  (`"cell": {"value": "...", "row": {"A2": "...", "D2": "..."}}`; `row` keys
  come in column order and the object may be empty). When the anchored cell
  holds a date or time, `cell` also carries `"raw"` — the machine-readable
  value behind the formatted display: `"2026-08-31 00:00:00"` for
  date-bearing cells, `"13:05:00"` for time-only cells (never a fictional
  epoch date), and elapsed `"36:00:00"` for `[h]`-style durations. The rare
  cell an xlsx stores as ISO 8601 text (`t="d"`) passes that string through
  verbatim (`"2026-08-31T13:05:00"`-shaped). `raw` is
  absent on every other cell kind, so its presence also identifies date
  cells. It is computed from the
  workbook at list time and is **never stored in the sidecar**; writers must
  ignore a `cell` key on input. When the workbook cannot be read (corrupt
  file) or the sheet was renamed, `cell` is omitted for the affected threads
  and the command still succeeds; a document path that does not exist at all
  is still an error, as for every `comment` command. A merged anchor's
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
