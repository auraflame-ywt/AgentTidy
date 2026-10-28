# antigravity local Windows capture

Captured 2026-10-28 from the local Windows install
(`%USERPROFILE%\.gemini\antigravity`).

Observed on this host, and therefore recorded here:

- The state root, the index file (`conversation_summaries.db`) and the
  three verified trees (`conversations/<UUID>.pb`, `brain/<UUID>/...`,
  `annotations/`) are **structurally identical** to the macOS capture.
- `conversation_summaries` holds the same columns; the projection query
  used by the adapter runs unchanged against the Windows database.
- **`title` is an empty string** for every row on this host (macOS rows
  carried real titles). An empty title must degrade to "no title", not
  render as a blank title — `row-windows.json` pins that shape.
- `workspace_uris` is a JSON array of `file:` URIs, but with a
  **Windows drive letter**: `file:///c%3A/Sample/project`. The adapter's
  `Url::to_file_path()` decodes it to `c:\Sample\project`.
- `status` is `CASCADE_RUN_STATUS_IDLE`, the same value observed on
  macOS — it still proves nothing about cleanability, so the lifecycle
  stays `Unknown` on both platforms.

Sanitization: real table DDL and metadata projection; the conversation
id, workspace URI and timestamp are redacted; `preview`, `raw_summary`
and the FTS tables are excluded. No credentials and no message bodies
are retained.
