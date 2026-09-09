# Agent Instructions

## Product

This repository contains an original Windows-first instant-replay clipping
application.

OBS Studio and similar products may be used only as behavioral references.
They are not implementation dependencies.

## Prohibited actions

Do not:

- Copy, translate, adapt, or paraphrase OBS source code.
- Link against or import libobs.
- Reproduce OBS internal classes, identifiers, or module structure.
- Copy OBS assets, UI, branding, comments, or documentation.
- Use code from OBS issues, patches, or pull requests.
- Introduce GPL or similarly restrictive dependencies without approval.
- Implement process injection or graphics hooks during the MVP.
- Place real-time media processing in the UI.
- Claim hardware behavior is verified unless it was actually tested.

## Engineering requirements

- Prefer official Microsoft, FFmpeg, codec, and hardware-vendor documentation.
- Keep capture, encoding, replay buffering, and muxing behind independent
  interfaces.
- Use bounded channels and bounded buffers.
- Do not block capture or encoding threads on disk or UI work.
- Add deterministic tests for timestamp and replay-buffer behavior.
- Record dependency names, versions, licenses, and purposes.
- Record major decisions in architecture decision records.
- Run formatting, linting, and tests before completing a task.
- Report all untested assumptions and known limitations.

## Workflow

For each task:

1. Restate the scope.
2. Inspect the existing architecture.
3. Propose a short implementation plan.
4. Implement the smallest complete change.
5. Add or update tests.
6. Run validation commands.
7. Summarize changed files, results, and remaining risks.
