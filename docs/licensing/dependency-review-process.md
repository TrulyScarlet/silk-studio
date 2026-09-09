# Dependency review process

Every dependency addition follows this checklist (spec §26, §30 rule 3):

1. Record name, version, source URL, license, linking mode, distributed
   artifacts, required notices, and purpose in the crate manifest comment
   *and* in `THIRD_PARTY_LICENSES.md`.
2. Confirm the license is permissive (MIT/Apache-2.0/BSD/Zlib family).
   Anything copyleft — especially GPL-family FFmpeg builds — requires
   explicit maintainer approval **before** it compiles into shipped
   artifacts (spec §30 rule 14; AGENTS.md prohibition). The exact Gyan FFmpeg
   build recorded in `THIRD_PARTY_LICENSES.md` has that approval.
3. Prefer crates already in the workspace dependency graph over new ones.
4. Re-verify resolved versions against the inventory at release time
   (Segment S9) using `Cargo.lock` / `package-lock.json`.

## Current status

All S0 dependencies remain recorded in `THIRD_PARTY_LICENSES.md` as
*Provisional*: MIT OR Apache-2.0 licensed Rust/JS tooling only. The exact Gyan
FFmpeg 9.0.1 full build is now explicitly approved as a GPLv3 external
process; its crate (`encoder-ffmpeg`) still has no FFmpeg library linkage or
bundled binary, and the application call site remains a separate integration
task.

The official [FFmpeg legal guidance](https://ffmpeg.org/legal.html) confirms
that the core is LGPLv2.1-or-later while optional GPL parts can apply the GPL
to the whole FFmpeg build. It also makes the configure flags, dynamic-linking
arrangement, corresponding source, and notices part of the compliance work;
the approved Gyan build and its exact hashes are recorded in the inventory.
