# Owned legacy DXGI factory evidence

ADR 0072. Actual owned fixtures on this Windows machine; no game targeting.

- All-feature prototype Clippy passed.
- Seven bootstrap native tests passed together, including existing modern factory
  lifecycle/soaks and new WARP/hardware legacy CreateSwapChain lifecycles.
- Both legacy paths observed two renderer creations and claimed only the selected
  HWND; tested pressure, retained pixels, fresh window-action content and renderer
  continuation after worker cancellation.
- Hardware legacy replay saved/decoded an 833-ms clip, checking fourteen primary-
  color samples. NVIDIA H.264 MFT reported fifteen GPU inputs, zero encoder
  readbacks and zero capture readbacks.
- WARP/software legacy replay saved/decoded an 841-ms clip, checking twenty-five
  primary-color samples; software encoding reported thirty-eight readbacks.

The first hardware lifecycle attempt failed a fixed 160-ms wall-time assumption
about how many unrelated-window Presents should have elapsed. The assertion now
waits up to three seconds for both actual renderer counters to advance twelve
times while capture stays saturated. It still requires both chains to progress;
it does not lower the required count or turn a timeout into a pass.

These tests establish legacy creation observation in exact opted-in fixtures, not
general attachment, pre-existing-chain discovery or real-game/fullscreen support.
No DLL loading into third-party games, release, install or signing was performed.
