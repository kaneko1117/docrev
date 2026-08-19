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
| `version` | int | Schema version. Currently `1`; readers must reject other values |
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

## CLI

`docrev comment` is the intended way for agents to read and write this file:

- `list --json` prints this document shape (`{"version": 1, "comments": [...]}`)
  after applying filters — the schema above is the output contract, plus one
  **derived, output-only** addition: each thread carries a `cell` object with
  the anchored cell's displayed text and its row's other non-empty cells
  (`"cell": {"value": "...", "row": {"A2": "...", "D2": "..."}}`; `row` keys
  come in column order and the object may be empty). It is computed from the
  workbook at list time and is **never stored in the sidecar**; writers must
  ignore a `cell` key on input. When the workbook cannot be read (corrupt
  file) or the sheet was renamed, `cell` is omitted for the affected threads
  and the command still succeeds; a document path that does not exist at all
  is still an error, as for every `comment` command. A merged anchor's
  `value` is its region's value, and the region's cells never repeat in
  `row`. Never redirect this output onto the sidecar itself — the shell
  truncates the file before the command reads it.
- `add` / `reply` / `resolve` print the affected thread (same thread shape,
  including its `id`) and exit non-zero with a message on stderr for invalid
  cell references, unknown sheets, or unknown thread ids.
- Writers hold an exclusive advisory lock on `<sidecar>.lock` during
  read-modify-write, so concurrent TUI and CLI writes cannot lose updates.
  The lock file is left in place; it is safe to delete when nothing is running.
- Threads whose `resolved` is `false` are shown with a `●` marker in the viewer.
- A missing sidecar file means "no comments" and is not an error. A corrupt or
  unsupported sidecar must not prevent opening the document read-only.
