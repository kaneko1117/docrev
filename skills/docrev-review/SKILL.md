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
   `anchor` (`{"sheet": "売上", "cell": "B3"}`), `author`, `body`, `created_at`,
   `replies` — and `cell`: the anchored cell's displayed text plus its row's
   other non-empty cells (`{"value": "...", "row": {"A3": "...", "C3": "..."}}`;
   `row` may be `{}`, and `cell` itself is absent when the workbook cannot be
   read). Never redirect this output onto the sidecar file itself.

3. For each thread: investigate, act, reply. **Start from the `cell` content
   that came with the thread** — for most comments the anchored row is all the
   context needed. Reach for the full sheet only when it is not:

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

5. Your replies appear in the user's open viewer automatically — no action needed on their side.

## Proactive findings

To flag something the user did not ask about, open a new thread:

```bash
docrev comment add <file.xlsx> --cell "Sheet1!B3" --body "..." --author claude
```

## Keep replies short

Comments are read in a narrow sidebar, a few characters wider than a phone
screen. Write for that space:

- **Two or three short sentences.** Lead with the answer, not the reasoning.
- One decision or fact per reply. Split unrelated points into their own threads.
- No headings, no bullet lists, no code blocks — they wrap badly in the panel.
- Reference cells by their address (`C5`), not by quoting their contents.
- When something needs a long explanation, say the conclusion in the thread and
  give the detail in the chat, where the user is already talking to you.

Good: `Checked the source data — 150 is current. Updated C5 and resolved.`
Too long: a paragraph explaining where the number came from, why it changed,
and what else it affects.

## Rules

- Always pass `--author claude` (or your agent's name) so the user can tell who
  wrote what; `user` is reserved for the human reviewer.
- Mutating commands print the affected thread as JSON — reuse the returned `id`.
- Errors exit non-zero with hints on stderr (an unknown sheet lists the
  available sheet names; cell references use the `Sheet!B3` form).
- Filters for `list`: `--unresolved`, `--author <name>`, `--sheet <name>`.
- Replying to a resolved thread reopens it. If the user answers a thread you
  just closed, it comes back to you on the next `list --unresolved`.
- The full sidecar schema is documented in `docs/sidecar.md` of the docrev
  repository.
