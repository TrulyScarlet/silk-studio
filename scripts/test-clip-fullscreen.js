// Run in the collaborative browser on Vite /?preview=1, via preview_evaluate.
// Optional window.silkTestVideoUrl supplies a local test video. No native IPC runs.
(async () => {
  if (!location.search.includes('preview=1') || window.__TAURI_INTERNALS__) {
    throw new Error('Use a fresh browser preview, never the installed recorder.');
  }
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const until = async (condition) => {
    for (let i = 0; i < 100; i++) { if (condition()) return; await wait(30); }
    throw new Error('Timed out waiting for player state');
  };
  const button = (label) => document.querySelector(`[aria-label="${label}"]`);
  const click = async (element) => { assert(element, 'Missing control'); element.click(); await wait(100); };
  const press = async (code) => { (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent('keydown', { code, bubbles: true })); await wait(100); };
  let fullscreen = false;
  const calls = [];
  // Background preview tabs do not reliably advance CSS transition frames.
  const deterministicMotion = document.createElement('style');
  deterministicMotion.textContent = '.clip-playback-chrome, .clip-fullscreen-topbar, .clip-edge-arrow { transition: none !important; }';
  document.head.append(deterministicMotion);
  window.__TAURI_INTERNALS__ = {
    convertFileSrc: () => window.silkTestVideoUrl ?? 'http://127.0.0.1:7991/clip.mp4',
    invoke: async (command, args) => {
      if (command === 'app_window_is_fullscreen') return fullscreen;
      if (command === 'app_window_set_fullscreen') { fullscreen = args.fullscreen; calls.push(fullscreen); return; }
      if (command === 'get_clip_keyframes') return [0, 2, 4];
      if (command === 'get_clip_audio_tracks') return [{trackId:1,name:'Game',channels:2,sampleRate:48000}];
      throw new Error('Isolated preview fixture');
    },
  };
  const assertFullscreen = () => {
    assert(fullscreen, 'Native fullscreen state');
    const stage = document.querySelector('.is-fullscreen-stage').getBoundingClientRect();
    const video = document.querySelector('video').getBoundingClientRect();
    for (const rect of [stage, video]) {
      assert(Math.abs(rect.x) < 1 && Math.abs(rect.y) < 1, 'Fullscreen starts at viewport origin');
      assert(Math.abs(rect.width - innerWidth) < 1 && Math.abs(rect.height - innerHeight) < 1, 'Entire viewport belongs to video');
    }
    assert(getComputedStyle(document.querySelector('video')).objectFit === 'contain', 'Video must not crop');
    assert(![...document.querySelectorAll('.clip-controls-bar button')].some((b) => b.textContent === 'Trim Clip'), 'Editing controls stay outside fullscreen');
    for (const selector of ['.vod-review-panel', '.clip-modal-header', '.audio-mixer-drawer']) {
      assert(!document.querySelector(selector)?.getClientRects().length, `${selector} must be hidden`);
    }
    assert(document.elementFromPoint(5, 5).closest('.clip-player-root'), 'Player covers custom titlebar');
  };
  try {
    await click(document.querySelector('[aria-label^="Preview "]'));
    await until(() => document.querySelector('video')?.readyState >= 1);
    // Keyboard fullscreen works while paused, including repeated F and Escape.
    await press('KeyF'); assertFullscreen();
    await press('KeyF'); assert(!fullscreen, 'F exits while paused');
    await click([...document.querySelectorAll('button')].find((b) => b.textContent === 'Theater Mode'));
    assert(document.querySelector('.is-theater-layout'), 'Theater enabled');
    await click(button('Fullscreen')); assertFullscreen();
    assert(!document.querySelector('.is-theater-layout'), 'Theater constraints suspended');
    const title = document.querySelector('.clip-modal-title-group h2').textContent;
    const nativeCalls = calls.length;
    assert(!button('Previous clip'), 'First clip has no previous arrow');
    const oldVideo = document.querySelector('video');
    oldVideo.muted = true; await oldVideo.play();
    await click(button('Next clip'));
    await until(() => document.querySelector('video') !== oldVideo && document.querySelector('video')?.readyState >= 1);
    assertFullscreen();
    assert(calls.length === nativeCalls, 'Navigation must not exit/reenter fullscreen');
    assert(oldVideo.paused && !oldVideo.getAttribute('src'), 'Old video is released');
    assert(document.querySelector('video').currentTime === 0, 'Next clip starts at zero');
    assert(document.querySelector('.clip-modal-title-group h2').textContent !== title, 'Next clip changes selection');
    await click(button('Previous clip'));
    assert(document.querySelector('.clip-modal-title-group h2').textContent === title, 'Previous clip restores selection');
    let visited = 1;
    while (button('Next clip')) { await click(button('Next clip')); assertFullscreen(); assert(++visited <= 5, 'No wrap or navigation loop'); }
    assert(visited >= 3, 'Multiple clips traversed');
    document.querySelector('.is-fullscreen-stage').dispatchEvent(new PointerEvent('pointermove', {bubbles:true}));
    await wait(2500);
    const chrome = document.querySelector('.clip-playback-chrome');
    await until(() => getComputedStyle(chrome).opacity === '0');
    assert(getComputedStyle(chrome).opacity === '0', `Controls hide after inactivity: ${JSON.stringify({class:chrome.parentElement.className,opacity:getComputedStyle(chrome).opacity,hover:chrome.matches(':hover'),focus:chrome.matches(':has(:focus-visible)')})}`);
    button('Previous clip').focus();
    await until(() => getComputedStyle(button('Previous clip')).opacity === '1');
    assert(getComputedStyle(button('Previous clip')).opacity === '1', 'Keyboard focus reveals arrow');
    await press('KeyF');
    assert(!fullscreen && document.querySelector('.is-theater-layout'), 'F restores theater');
    assert(document.querySelector('.vod-review-panel').getClientRects().length, 'Review returns after fullscreen');
    await press('Escape');
    assert(!document.querySelector('.clip-player-root'), 'Escape closes player');
    return {passed:true, visited, nativeTransitions:calls, viewport:[innerWidth,innerHeight]};
  } finally {
    if (button('Close clip player')) await click(button('Close clip player'));
    delete window.__TAURI_INTERNALS__;
    deterministicMotion.remove();
  }
})()
