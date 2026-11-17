# Retired remote epoch with a pending producer-copy removal

ADR 0079. Exact owned integrated worker only; no actual game or independent
remote DLL helper is targeted. No TDR, injection into an unrelated process,
driver/security setting change or capture fallback.

## Current targeted results

- Hardware native: two same-HWND removal/recreation epochs, 24 checked new GPU
  publications, increasing QPC, retained old local pixels, exact new approval and
  continued forwarding after capture stop pass.
- Hardware RecorderEngine: three MP4s saved and approved-FFmpeg decoded across
  318x178, 640x360 and 800x450. The shared replay test asserts zero encoder
  readbacks on hardware and zero reported capture readbacks.
- Six independent hardware cold starts pass a controlled transition after one
  exact initial ACK, normal exit and retained fresh pixels after target exit.
- Stale ACK/request, wrong retirement/approval and missing retirement/approval
  fail closed with **normal error exit 1**, not merely any unsuccessful status.
- Hardware before-first-ACK removal request is explicitly unsupported and exits
  1 without removal or a replacement offer. No guessed readiness/timing sleep.
- Stop during retirement resumes only the still-healthy owned renderer so the
  forwarder can stop normally. Stop during fresh approval also exits normally.
  Retained old local red pixels survive both cancellation points.
- WARP is rejected **before graphics startup** with normal error exit 1 for this
  new route. Existing healthy/quiescent/local WARP tests are separate.

## Important failed attempts

Early WARP positive native/replay attempts passed in isolation, but subsequent
negative/cold-start tests found intermittent native exception 0xc0000005 during
removed-graph resource teardown. It also recurred after a valid initial ACK.
Closing original exported handles first, dependency-preserving destruction order
and proving earlier queue-tail completion did not eliminate it. Its root cause
remains unconfirmed. Do not turn isolated WARP passes into compatibility claims.

The opt-in owned-fixture exception diagnostic records fault address and bounded
stack candidates using standard Windows user-mode APIs, continuing normal fatal
handling. It is not installed in production apps or third-party targets. No OS
crash configuration or security control was changed. Adding the windows 0.62.2
Win32_System_Kernel binding feature exposes exception context types only, not a
kernel driver or privileged operation; no external package/version was added.

## Scope and remaining gates

The parent retires and drops old imported resources **before** the private
producer copy is deliberately held pending and removed. This is cross-process
retirement/reconstruction with retained independent local/encoder textures, not
unexpected loss racing active parent copies. Independent DLL-worker recovery,
automatic application graph discovery, real games/fullscreen, audio/endurance
and packaged release acceptance remain unfinished. Full combined regression
after these changes is pending; targeted results are not a full-feature claim.

The subsequent combined tracked native regression **passed with actual exit 0**,
12:41:35–13:00:32 UTC: **65 recorder, 23 attachment and 30 device-lifetime tests**,
plus previous native suites. Record:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-remote-pending-copy-c6d8d51bd9954c868b3cfd89a7713d2b`.
This validates the restricted hardware path and normal WARP rejection, not the
earlier unsafe WARP attempts. Later ADR 0080 changes require their own regression.
