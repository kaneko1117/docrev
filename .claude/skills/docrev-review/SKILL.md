---
name: docrev-review
description: Handle review comments left on Excel files via docrev. Use when the user says they commented on a spreadsheet in docrev, or asks you to review or answer comments in an .xlsx file.
---

# docrev review workflow

docrev stores review comments in a sidecar JSON file (`<file>.xlsx.docrev.json`)
next to the document. docrev itself never modifies the document — when a fix
belongs in the file, you make it with your own tools (see "Editing the
workbook"). Interact with comments through the `docrev comment` CLI — never
edit the sidecar by hand; the CLI locks and writes atomically.

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

   A second array, `workbook_comments`, carries the workbook's own Excel
   comments (notes and threaded comments). They are **read-only context**:
   they have no `id`, and `reply`/`resolve` can never target them. To answer
   one, add a docrev thread on the same cell instead.

3. For each thread: investigate, act, reply. **Start from the `cell` content
   that came with the thread** — for most comments the anchored row is all the
   context needed. Reach for the full sheet only when it is not:

   ```bash
   docrev dump <file.xlsx> --sheet <name>
   docrev dump <file.xlsx> --sheet <name> --formulas   # when asked to check a total or a formula
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

5. The user's open viewer picks up your replies automatically: the cell's `●`
   marker reappears, and pressing `c` on it shows the thread (on terminals
   wide enough for the side panel). No action needed on your side.

## Proactive findings

To flag something the user did not ask about, open a new thread:

```bash
docrev comment add <file.xlsx> --cell "Sheet1!B3" --body "..." --author claude
```

## Editing the workbook

docrev has no write commands — editing the document is your job, with your own
tools (e.g. Python + openpyxl). When a comment asks for a fix in the file
itself:

1. **Copy the file first** (`cp file.xlsx file.xlsx.bak`) unless it is tracked
   by git — the copy is the only undo there is.
2. **Do not edit while Excel has the file open** (a `~$<name>.xlsx` file sits
   next to it): Excel's next save would erase your change.
3. **Mind regeneration loss.** Libraries rewrite the whole workbook on save;
   drawings, shapes, pivot tables and other parts they do not model can be
   dropped. If the file contains such parts, confirm with the user before the
   first edit.
4. **Record every edit on its thread** — reply with the cell address and
   before → after, then resolve. An edit that is not in the comments did not
   happen. Batch one thread's edits into one reply.

Good: `Fixed E10: 「アカウントがロックされています」 → 「IDまたはパスワードが違います」. Resolved.`

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
