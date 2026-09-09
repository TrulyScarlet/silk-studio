# ADR 0013: Initial clip-library catalog

**Status:** Accepted for the initial S7 slice

## Context

The desktop library needs to remember clips that disappear from the output
directory so the UI can distinguish a missing file from a clip that has never
been indexed. The filesystem remains authoritative for completed media, and
library scans and file actions must not run on capture, encoding, or UI render
threads.

The implementation plan mentions SQLite for the complete S7 library. The first
usable slice only needs bounded listing, reconciliation, and file actions; it
does not yet need query pagination, thumbnails, or a 1,000-row stress path.

## Decision

Use a versioned JSON catalog beside the application configuration file for the
initial slice. The catalog stores only index metadata: path, display name,
creation time, duration when readable, size, missing status, and the user's
protection flag. The configured output directory is scanned for completed
`.mkv` and `.mp4` files; `.part` artifacts and other files are ignored. Existing
catalog rows are retained as missing when their files are absent, while files
found on disk are added or refreshed.

All scans, catalog writes, renames, deletes, protection changes, storage policy
operations, and system file actions run on the bounded `silk-library-worker`.
The worker is reached through a queue bounded to eight requests. Rename, delete,
protection, and opener actions validate indexed paths against the configured
output directory; opener actions additionally reject links and paths whose
canonical parent escapes that directory. Automatic deletion is opt-in and
removes oldest present, unprotected clips until the configured quota is met.

## Consequences

- No new third-party dependency or native database runtime is required.
- Catalog writes are small and atomic within the same directory where the
  temporary catalog is written.
- The catalog is not media storage and can be rebuilt from the filesystem;
  missing rows are retained only to provide useful user feedback.
- SQLite, pagination, filtering, thumbnails, and embedded playback remain later
  work and must be introduced with a new decision if needed.
