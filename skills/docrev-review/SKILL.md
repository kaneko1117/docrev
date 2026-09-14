---
name: docrev-review
description: Handle review comments left on Excel or Markdown files via docrev. Use when the user says they commented on a spreadsheet or a Markdown file in docrev, or asks you to review or answer comments in an .xlsx or .md file.
---

# docrev review workflow

docrev stores review comments in a sidecar JSON file (`<file>.docrev.json`, e.g.
`budget.xlsx.docrev.json`) next to the document. docrev itself never modifies
the document — when a fix belongs in the file, you make it with your own tools
(see "Editing the workbook" and "Markdown files"). Interact with comments
through the `docrev comment` CLI — never edit the sidecar by hand; the CLI
locks and writes atomically.

## When the user says "I commented"

1. Identify the document path (ask if ambiguous).
2. Read the open threads:

   ```bash
   docrev comment list <file> --json --unresolved
   ```

   The output is `{"version": 2, "comments": [...]}`. Each thread carries `id`,
   `anchor` (`{"sheet": "売上", "cell": "B3"}`), `author`, `body`, `created_at`,
   `replies` — and `cell`: the anchored cell's displayed text plus its row's
   other non-empty cells (`{"value": "...", "row": {"A3": "...", "C3": "..."}}`;
   `row` may be `{}`, and `cell` itself is absent when the workbook cannot be
   read). A Markdown file's threads are shaped differently — see "Markdown
   files". Never redirect this output onto the sidecar file itself.

   A second array, `workbook_comments`, carries the workbook's own Excel
   comments (notes and threaded comments). They are **read-only context**:
   they have no `id`, and `reply`/`resolve` can never target them. To answer
   one, `comment add` on the same cell instead.

3. For each thread: investigate, act, reply. **Start from the `cell` content
   that came with the thread** — for most comments the anchored row is all the
   context needed. Reach for the full sheet only when it is not:

   ```bash
   docrev dump <file.xlsx> --sheet <name>
   docrev dump <file.xlsx> --sheet <name> --formulas   # when asked to check a total or a formula
   ```

   Then answer on the thread:

   ```bash
   docrev comment reply <file> --thread <id> --body "..." --author claude
   ```

4. Resolve threads that are fully handled:

   ```bash
   docrev comment resolve <file> --thread <id>
   ```

   Leave a thread open when you need the user's decision — say so in your reply.

5. The user's open viewer picks up your replies automatically: the cell's or
   line's `●` marker reappears, and on terminals wide enough for the side panel
   the thread shows there (on a workbook after pressing `c` on the cell; on a
   Markdown file whenever the cursor is on the line). No action needed on your
   side.

## Proactive findings

To flag something the user did not ask about, comment on its cell or line:

```bash
docrev comment add <file.xlsx> --cell "Sheet1!B3" --body "..." --author claude
docrev comment add <file.md> --line 13 --body "..." --author claude
```

A cell or a line holds one thread: when it already has one, `add` appends to it
(reopening it if it was resolved) and prints that thread, not a new one.

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

## Markdown files

On a `.md` file a thread is anchored to a line:
`"anchor": {"kind": "line", "line": 13}`, 1-based — the number `cat -n` and
the viewer show. Instead of `cell` it carries `line`: the line's text and the
two lines on each side, keyed by line number
(`{"text": "brew install docrev", "context": {"11": "## Install", "12": "", "14": "", "15": "Check it works:"}}`).

1. **Start from `line.context`.** Read the whole file (`docrev dump <file.md>`,
   or `cat -n`) only when the surrounding lines are not enough.
2. A thread carrying `"hidden": true` and no `line` points past the end of the
   file: the file got shorter since the comment was written. Ask the user what
   it was about rather than guessing.
3. **Edit the file with your own tools**, and record each edit on its thread
   as for a workbook: line number and before → after.
4. **Comments do not move with the text.** A thread on line 13 stays on line 13
   after you insert or delete lines above it, so it then sits next to
   different text. Reply to and resolve every thread you handled; for a thread
   that stays open below an edit that shifted lines, give its text's new line
   number in your reply.
5. `--sheet` filters and `dump --sheet` / `--formulas` apply to workbooks only.

## Keep replies short

Comments are read in a narrow sidebar, a few characters wider than a phone
screen. Write for that space:

- **Two or three short sentences.** Lead with the answer, not the reasoning.
- One decision or fact per reply. A cell or line holds one thread, so
  unrelated points on it go into separate replies.
- No headings, no bullet lists, no code blocks — they wrap badly in the panel.
- Reference cells by their address (`C5`) and lines by number (`line 13`), not
  by quoting their contents.
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
  available sheet names; cell references use the `Sheet!B3` form; a well-formed
  `--cell` on a Markdown file or `--line` on a workbook names the right flag).
- Filters for `list`: `--unresolved`, `--author <name>`, `--sheet <name>`.
- Replying to a resolved thread reopens it. If the user answers a thread you
  just closed, it comes back to you on the next `list --unresolved`.
- The full sidecar schema is documented in `docs/sidecar.md` of the docrev
  repository.
