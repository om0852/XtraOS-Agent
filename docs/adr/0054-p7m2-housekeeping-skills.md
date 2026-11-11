# ADR 0054 — P7-M2 Desktop housekeeping: duplicates, largest files, organize, bulk rename

- Status: accepted (owner: "continue and start building future things", 2026-11-11; plan row
  P7-M2: sort Downloads by rule, bulk rename, find duplicates, tidy desktop).
- Date: 2026-11-11

## Context
Agents already have `files.list/find/open/mkdir/copy/move/rename/delete` (ADR 0026): protected
places refused, every change through the Windows shell file operation (undoable, never
overwrites), deletes to the Recycle Bin after the user's OK. Housekeeping jobs ("clean up my
Downloads", "find duplicate photos", "what is filling my disk?") would take an agent hundreds of
calls with them. The health guard (ADR 0050) now tells agents when the disk is nearly full; they
need a way to see what takes the space.

## Decision
Four built-in skills (no new MCP tools):

1. **`files.duplicates {path, min_kb?: 100, limit?: 20}`** — read-only. Walks the folder and its
   subfolders like `files.find` (breadth-first, skips hidden, AppData, `node_modules`, links and
   protected places; 15 s budget). Files are duplicates only when **byte-for-byte identical**:
   same size → same first 64 KB (SHA-256) → full streamed comparison. Groups sorted by wasted
   space, newest copy first; the total wasted space. Deleting stays `files.delete` (asks the user).
2. **`files.largest {path, limit?: 20}`** — read-only: the biggest files under a folder (same
   walk), with sizes and dates.
3. **`files.organize {path, by?: "type" | "month", apply?: false}`** — the files directly in
   `path` (not subfolders) into subfolders: by type `Images, Documents, Videos, Audio, Archives,
   Installers, Code, Other`; by month `YYYY-MM` (last modified). Without `apply` it returns the
   plan only (read-only). With `apply: true` it is a file change (the user is asked with the
   summary), folders are created and files moved in one shell operation (undo restores them).
   Skipped, and listed: unfinished downloads (`.crdownload .part .partial .download .tmp`), files
   changed in the last 2 minutes, and names already present in the target folder.
4. **`files.bulk_rename {path, find, replace, pattern?: "*", apply?: false}`** — files directly in
   `path` whose name matches `pattern` and contains `find` (case-insensitive) get it replaced.
   Preview without `apply`; with `apply` a file change (asked), all renames in one shell
   operation. Refused before anything changes: a new name with `\ / : * ? " < > |`, an empty
   name, two files ending with the same name, or a name that already exists.

## Tests (built)
- Unit `housekeeping_rules`: type folders, unfinished downloads, the rename plan (case-insensitive,
  collisions, empty names, path characters).
- Unit `housekeeping_on_a_temp_folder`, real files and the real shell file operation: a 300 KB
  file and its copy in a subfolder are one duplicate group (a same-size file differing in the
  last byte is not); `files.largest` order; `files.organize` preview moves nothing and is policy
  class observe, `apply` is `file_change` with the plan as the approval text, then moves 4 files
  into Images/Documents/Installers and leaves the `.crdownload`; `files.bulk_rename` preview,
  apply, and refusal of a rename onto an existing name.
- The approval dialog itself is the existing policy path (ADR 0017/0026), not re-tested here.

## Live (owner's Downloads, read-only, 2026-11-11)
`files.largest` 0.5 s: 3,384 files, 148 GB. `files.duplicates` 20 s (budget reached): 15 groups,
2.7 GB could be freed. `files.organize` preview 0.2 s: 384 loose files into 8 folders, 5 skipped.
Nothing was moved.
