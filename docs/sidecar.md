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
  thread. Start a new thread for a new topic, even on the same cell.
- Threads whose `resolved` is `false` are shown with a `●` marker in the viewer.
- A missing sidecar file means "no comments" and is not an error. A corrupt or
  unsupported sidecar must not prevent opening the document read-only.
