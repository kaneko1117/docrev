---
name: docrev-review
description: Handle review comments left on Excel files via docrev. Use when the user says they commented on a spreadsheet in docrev, or asks you to review or answer comments in an .xlsx file.
---

# docrev review workflow

docrev stores review comments in a sidecar JSON file (`<file>.xlsx.docrev.json`)
next to the document. The original document is never modified. Interact through
the `docrev comment` CLI — never edit the sidecar by hand; the CLI locks and
writes atomically.

## When the user says "I commented"

1. Identify the document path (ask if ambiguous).
2. Read the open threads:

   ```bash
   docrev comment list <file.xlsx> --json --unresolved
   ```

   The output is `{"version": 1, "comments": [...]}`. Each thread carries `id`,
   `anchor` (`{"sheet": "売上", "cell": "B3"}`), `author`, `body`, `created_at`
   and `replies`.

3. For each thread: investigate, act, reply. Read the data with:

   ```bash
   docrev dump <file.xlsx> --sheet <name>
   ```

   Then answer on the thread:

   ```bash
   docrev comment reply <file.xlsx> --thread <id> --body "..." --author claude
   ```

4. Resolve threads that are fully handled:

   ```bash
   docrev comment resolve <file.xlsx> --thread <id>
   ```

   Leave a thread open when you need the user's decision — say so in your reply.

5. Tell the user to press `F5` in the docrev viewer to see your replies.

## Proactive findings

To flag something the user did not ask about, open a new thread:

```bash
docrev comment add <file.xlsx> --cell "Sheet1!B3" --body "..." --author claude
```

## Rules

- Always pass `--author claude` (or your agent's name) so the user can tell who
  wrote what; `user` is reserved for the human reviewer.
- Mutating commands print the affected thread as JSON — reuse the returned `id`.
- Errors exit non-zero with hints on stderr (an unknown sheet lists the
  available sheet names; cell references use the `Sheet!B3` form).
- Filters for `list`: `--unresolved`, `--author <name>`, `--sheet <name>`.
- The full sidecar schema is documented in `docs/sidecar.md` of the docrev
  repository.
