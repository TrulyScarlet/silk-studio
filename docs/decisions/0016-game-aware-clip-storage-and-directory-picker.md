# ADR 0016: Game-aware clip storage and directory picker

**Status:** Accepted for the S7 game-aware storage slice

## Context

The clip directory must be user-selectable, including a drive root such as
`E:\`, without allowing the recorder to scan an entire drive. Saved clips also
need a stable library grouping that can be recovered from the filesystem rather
than from UI-only state.

## Decision

Store the user-selected location as a base directory in `output.directory`.
Silk owns only the managed child directory `<base>\Silk`; the controller saves
new files below `<base>\Silk\<game>`, creating the game directory on demand.
When no foreground process can be attributed, the controller uses
`Uncategorized`.

On Windows, the shell observes the foreground process during command polling
and save requests. It records only the executable stem of the most recent
non-Silk foreground process. This is a best-effort attribution mechanism: it
does not inject into games, install a game database, or transmit telemetry.

The library worker receives the managed `Silk` root and scans only root-level
clips plus files directly inside one child directory. It derives `gameName`
from that child directory, retains root files as legacy/uncategorized entries,
and rejects deeper or reparse-point paths.

The desktop shell uses the official `tauri-plugin-dialog` native folder picker
through a narrow `pick_clip_directory` command. The picker returns a path only;
settings validation and persistence remain owned by the existing configuration
boundary.

## Consequences

- Users can move the base directory without exposing unrestricted filesystem
  traversal to the frontend.
- The managed folder has a predictable, portable layout:
  `<base>\Silk\<game>\Clip_...mkv`.
- Existing flat clips already under the managed root remain visible as
  uncategorized entries; files outside the selected base are not imported.
- Process names are not guaranteed to be game titles. Unknown, inaccessible,
  or non-game foreground applications may be grouped by executable name and
  require later classification improvements.
- `tauri-plugin-dialog` 2.7.2 and its transitive native dialog dependencies
  must remain in the license inventory and provisional approval process.
