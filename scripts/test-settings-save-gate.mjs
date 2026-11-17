// Dependency-free Windows Edge regression test. Run: node scripts/test-settings-save-gate.mjs
// Uses isolated browser storage and a fake IPC bridge, never the installed recorder.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const profile = await mkdtemp(join(process.env.LOCALAPPDATA, "Temp", "opencode", "silk-settings-"));
const frontend = resolve("apps/desktop");
const vite = spawn(process.execPath, [join(frontend, "node_modules/vite/bin/vite.js"), "--host", "127.0.0.1", "--port", "5174"], { cwd: frontend, stdio: "ignore" });
const edge = spawn(process.env.SILK_TEST_EDGE ?? "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", ["--headless=new", "--no-first-run", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
let socket;
try {
  const endpoint = await new Promise((resolveEndpoint, reject) => {
    const timeout = setTimeout(() => reject(new Error("Edge debugger startup timed out")), 20_000);
    let output = "";
    edge.on("error", reject);
    edge.stderr.on("data", (chunk) => {
      output += chunk;
      const match = output.match(/DevTools listening on (ws:\/\/[^\s]+)/);
      if (match) { clearTimeout(timeout); resolveEndpoint(match[1]); }
    });
  });
  const origin = new URL(endpoint).origin.replace("ws:", "http:");
  const targets = await (await fetch(`${origin}/json/list`)).json();
  socket = new WebSocket(targets.find((target) => target.type === "page").webSocketDebuggerUrl);
  await new Promise((ready) => socket.addEventListener("open", ready, { once: true }));
  let sequence = 0;
  const pending = new Map();
  socket.addEventListener("message", ({ data }) => {
    const message = JSON.parse(data);
    const entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);
    clearTimeout(entry.timeout);
    if (message.error) entry.reject(new Error(JSON.stringify(message.error)));
    else entry.resolve(message.result);
  });
  const command = (method, params = {}) => new Promise((resolveResult, reject) => {
    const id = ++sequence;
    const timeout = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 10_000);
    pending.set(id, { resolve: resolveResult, reject, timeout });
    socket.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (expression) => {
    const result = await command("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const until = async (expression) => {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (await evaluate(`Boolean(${expression})`)) return;
      await delay(50);
    }
    throw new Error(`Condition timed out: ${expression}`);
  };
  const click = async (selector) => { await evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`); };
  await command("Page.enable");
  await command("Page.addScriptToEvaluateOnNewDocument", { source: `
    window.testSaves = []; window.testIcons = []; window.failSave = false;
    window.testCallbacks = {}; window.testListeners = {}; window.testCallbackId = 0;
    window.emitController = (payload) => { const id = window.testListeners['controller-event']; window.testCallbacks[id]?.({event:'controller-event',id:1,payload}); };
    window.__TAURI_INTERNALS__ = {
      transformCallback: (callback) => { const id = ++window.testCallbackId; window.testCallbacks[id] = callback; return id; }, unregisterCallback: () => {},
         invoke: async (command, args) => {
          if (command === 'get_settings' && sessionStorage.testSettings) return JSON.parse(sessionStorage.testSettings);
         if (command === 'get_display_sources') return [
           { id: 'display-1', name: 'Primary', is_primary: true, width: 1920, height: 1080 },
           { id: 'display-2', name: 'Second monitor', is_primary: false, width: 2560, height: 1440 },
         ];
         if (command === 'get_window_sources') return [
           { id: 'pid:42:hwnd:120', title: 'Game fixture', appName: 'game-fixture' },
           { id: 'pid:43:hwnd:121', title: 'Background game', appName: 'background-fixture' },
         ];
        if (command === 'configure_settings') {
          if (window.failSave) throw new Error('test save failure');
           window.testSaves.push(structuredClone(args.settings)); sessionStorage.testSettings = JSON.stringify(args.settings); return;
        }
        if (command === 'update_theme_icon') { window.testIcons.push(args.theme); return; }
         if (command === 'plugin:event|listen') { window.testListeners[args.event] = args.handler; return 1; }
        throw new Error('preview fixture fallback');
      }
    };
  ` });
  for (let attempt = 0; attempt < 100; attempt++) {
    try { if ((await fetch("http://127.0.0.1:5174")).ok) break; } catch { /* startup */ }
    await delay(50);
  }
  await command("Page.navigate", { url: "http://127.0.0.1:5174/?preview=1" });
  await until(`document.querySelector('[aria-label="Open Settings dialog"]') && document.body.textContent.includes('5 Clips')`);
  const open = async () => {
    await click('[aria-label="Open Settings dialog"]');
    await until(`document.querySelector('[role="dialog"]')`);
    await evaluate(`Array.from(document.querySelectorAll('button')).find(b => b.textContent.includes('Themes')).click()`);
    await until(`document.querySelectorAll('.theme-card').length === 4`);
  };
  const selectEmber = async () => {
    await evaluate(`Array.from(document.querySelectorAll('.theme-card')).find(b => b.textContent.includes('Autumn Ember')).click()`);
    await until(`document.querySelector('.theme-card.is-selected').textContent.includes('Autumn Ember')`);
  };
  const press = async (key, modifiers = 0, code = key) => {
    await command("Input.dispatchKeyEvent", { type: "keyDown", key, code, modifiers });
    await command("Input.dispatchKeyEvent", { type: "keyUp", key, code, modifiers });
  };
  // Every close path must throw away the selected draft, with no native side effects.
  for (const close of [
    () => press("Escape"),
    () => press(",", 2, "Comma"),
    () => evaluate(`Array.from(document.querySelectorAll('button')).find(b => b.textContent.trim() === 'Close').click()`),
    () => click('.settings-modal-close-btn'),
    () => click('.settings-modal-backdrop'),
  ]) {
    await open();
    await selectEmber();
    assert.equal(await evaluate(`document.documentElement.dataset.theme`), "studio");
    assert.equal(await evaluate(`window.testSaves.length`), 0);
    assert.equal(await evaluate(`window.testIcons.includes('ember')`), false);
    await close();
    await until(`!document.querySelector('[role="dialog"]')`);
    await open();
    assert.match(await evaluate(`document.querySelector('.theme-card.is-selected').textContent`), /Silk Studio/);
    await press("Escape");
  }
  await open();
  await selectEmber();
  await evaluate(`window.failSave = true; document.querySelector('form').requestSubmit()`);
  await until(`document.body.textContent.includes('test save failure')`);
  assert.equal(await evaluate(`document.documentElement.dataset.theme`), "studio");
  await evaluate(`window.failSave = false; document.querySelector('form').requestSubmit()`);
  await until(`document.documentElement.dataset.theme === 'ember'`);
  assert.equal(await evaluate(`window.testSaves.length`), 1);
  await press("Escape");
  await open();
  assert.match(await evaluate(`document.querySelector('.theme-card.is-selected').textContent`), /Autumn Ember/);
   await evaluate(`Array.from(document.querySelectorAll('.settings-tab-btn')).find(b => b.textContent.includes('Capture')).click()`);
   const select = async (selector, value) => {
     await evaluate(`(() => {
       const element = document.querySelector(${JSON.stringify(selector)});
       Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(element, ${JSON.stringify(value)});
       element.dispatchEvent(new Event('change', { bubbles: true }));
     })()`);
   };
    const modeSelector = '#capture-mode';
    assert.deepEqual(await evaluate(`Array.from(document.querySelector('#capture-mode').options, o => o.text)`), ['Game', 'Screen', 'Window']);
    const screenshot = async (name, widths = [1440, 768, 390], scrollSelector) => {
      if (!process.env.SILK_SETTINGS_SCREENSHOTS) return;
      await mkdir(process.env.SILK_SETTINGS_SCREENSHOTS, { recursive: true });
      for (const width of widths) {
        await command('Emulation.setDeviceMetricsOverride', { width, height: 900, deviceScaleFactor: 1, mobile: false });
        if (scrollSelector) await evaluate(`document.querySelector(${JSON.stringify(scrollSelector)}).scrollIntoView({block:'start'})`);
        await evaluate(`new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))`);
        await evaluate(`Promise.all(document.getAnimations().filter(a => a.effect?.getTiming().iterations !== Infinity).map(a => a.finished.catch(() => {})))`);
        const { data } = await command('Page.captureScreenshot', { format: 'png' });
        await writeFile(join(process.env.SILK_SETTINGS_SCREENSHOTS, `${name}-${width}.png`), Buffer.from(data, 'base64'));
      }
      await command('Emulation.clearDeviceMetricsOverride');
    };
   await select(modeSelector, 'display');
   await until(`document.querySelector('#capture-target-display')`);
    const monitor = 'display-2';
    await select('#capture-target-display', monitor);
    await screenshot('screen', [1440]);
   await evaluate(`document.querySelector('form').requestSubmit()`);
   await until(`window.testSaves.length === 2`);
   assert.equal(await evaluate(`window.testSaves.at(-1).capture.sourceType`), 'display');
   assert.equal(await evaluate(`window.testSaves.at(-1).capture.sourceId`), monitor);
   // A failed source save must not wedge subsequent submissions or discard the draft.
   await select(modeSelector, 'window');
   await until(`document.querySelector('#capture-target-window')`);
    const selectedWindow = 'pid:43:hwnd:121';
    await select('#capture-target-window', selectedWindow);
    await screenshot('window', [1440]);
   await evaluate(`window.failSave = true; document.querySelector('form').requestSubmit()`);
   await until(`document.body.textContent.includes('test save failure')`);
   await evaluate(`window.failSave = false; document.querySelector('form').requestSubmit()`);
   await until(`window.testSaves.length === 3`);
   assert.equal(await evaluate(`window.testSaves.at(-1).capture.sourceId`), selectedWindow);
    await select(modeSelector, 'game');
    await until(`document.querySelector('#capture-game-selection')?.value === 'automatic'`);
    assert.equal(await evaluate(`Boolean(document.querySelector('#capture-target-window'))`), false);
    assert.match(await evaluate(`document.querySelector('#capture-visibility-notice').textContent`), /not an isolated view/);
    assert.equal(await evaluate(`document.querySelector('.capture-help details').open`), false);
    await click('.capture-help summary');
    assert.equal(await evaluate(`document.querySelector('.capture-help details').open`), true);
    assert.match(await evaluate(`document.querySelector('.capture-help details').textContent`), /VALORANT.*CS2/);
    await click('.capture-help summary');
    await screenshot('game-automatic');
    await evaluate(`document.querySelector('form').requestSubmit()`);
    await until(`window.testSaves.length === 4`);
    assert.equal(await evaluate(`window.testSaves.at(-1).capture.sourceType`), 'game-auto');
    await select('#capture-game-selection', 'manual');
    await until(`document.querySelector('#capture-target-window')`);
    assert.equal(await evaluate(`document.querySelector('#capture-target-window').value`), selectedWindow);
    await screenshot('game-manual');
    await evaluate(`document.querySelector('form').requestSubmit()`);
    await until(`window.testSaves.length === 5`);
    assert.equal(await evaluate(`window.testSaves.at(-1).capture.sourceType`), 'game');
    // Closing without Save discards the auto/manual change as well as the target.
    await select('#capture-game-selection', 'automatic');
    await press('Escape');
    await open();
    await evaluate(`Array.from(document.querySelectorAll('.settings-tab-btn')).find(b => b.textContent.includes('Capture')).click()`);
    await until(`document.querySelector('#capture-game-selection')?.value === 'manual'`);
    assert.equal(await evaluate(`window.testSaves.length`), 5);
    // Old persisted DXGI selections render as Game / Choose a game, unchanged on Save.
    await evaluate(`(() => { const settings = JSON.parse(sessionStorage.testSettings); settings.capture.sourceType = 'game-guarded'; sessionStorage.testSettings = JSON.stringify(settings); })()`);
    await command('Page.reload');
    await until(`document.querySelector('[aria-label="Open Settings dialog"]')`);
    await open();
    await evaluate(`Array.from(document.querySelectorAll('.settings-tab-btn')).find(b => b.textContent.includes('Capture')).click()`);
    await until(`document.querySelector('#capture-game-selection')?.value === 'manual'`);
    assert.equal(await evaluate(`document.querySelector('#capture-mode').value`), 'game');
    assert.equal(await evaluate(`document.querySelector('#capture-target-window').value`), selectedWindow);
    await evaluate(`document.querySelector('form').requestSubmit()`);
    await until(`window.testSaves.length === 1`);
    assert.equal(await evaluate(`window.testSaves.at(-1).capture.sourceType`), 'game-guarded');
    assert.equal(await evaluate(`window.testSaves.at(-1).capture.sourceId`), selectedWindow);
    await evaluate(`Array.from(document.querySelectorAll('.settings-tab-btn')).find(b => b.textContent.includes('Themes')).click()`);
    const input = async (selector, value) => {
      await evaluate(`(() => { const element = document.querySelector(${JSON.stringify(selector)});
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(element, ${JSON.stringify(value)});
        element.dispatchEvent(new Event('input', {bubbles:true})); element.dispatchEvent(new Event('change', {bubbles:true})); })()`);
    };
    const button = async (text) => evaluate(`Array.from(document.querySelectorAll('button')).find(b => b.textContent.trim() === ${JSON.stringify(text)}).click()`);
    const stagePreset = async () => {
      await input('[aria-label="Main color"]', '#4db6c8');
      await input('[aria-label="Buffering and ready text"]', '#87cce2');
      await input('[aria-label="Clip saved text and badge"]', '#e7a26e');
      await input('[aria-label="Clip saved background"]', '#15161a');
      await input('[aria-label="Overlay accent"]', '#7fb7e6');
      await input('[aria-label="Overlay background"]', '#161c25');
      await input('[aria-label="Overlay text"]', '#dcecff');
      await select('#theme-icon', 'vamp');
      await input('#theme-preset-name', 'Ocean');
      await button('Save as preset');
      await until(`document.querySelector('#theme-preset').value === 'Ocean'`);
    };
    await stagePreset();
    assert.notEqual(await evaluate(`document.documentElement.style.getPropertyValue('--capture-color')`), '#87cce2');
    assert.equal(await evaluate(`window.testIcons.includes('vamp')`), false);
    await press('Escape'); await open();
    assert.equal(await evaluate(`document.querySelector('#theme-preset').options.length`), 1);
    await stagePreset();
    await evaluate(`window.failSave = true; document.querySelector('form').requestSubmit()`);
    await until(`document.body.textContent.includes('test save failure')`);
    assert.equal(await evaluate(`window.testIcons.includes('vamp')`), false);
    await evaluate(`window.failSave = false; document.querySelector('form').requestSubmit()`);
    await until(`document.documentElement.style.getPropertyValue('--capture-color') === '#87cce2'`);
    await until(`window.testIcons.at(-1) === 'vamp'`);
    assert.equal(await evaluate(`window.testSaves.at(-1).application.themePresets[0].name`), 'Ocean');
    assert.equal(await evaluate(`window.testSaves.at(-1).application.themePresets[0].iconTheme`), 'vamp');
    await evaluate(`document.querySelector('#theme-preset').scrollIntoView({block:'start'})`);
    await screenshot('custom-preset', [1440, 768, 390], '#theme-preset');
    await evaluate(`document.querySelector('.custom-color-grid').scrollIntoView({block:'start'})`);
    await screenshot('custom-colors', [1440, 768, 390], '.custom-color-grid');
    await press('Escape');
    await evaluate(`window.emitController({type:'status_changed',state:'Buffering'}); window.emitController({type:'clip_saved',path:'C:/fixture/replay.mp4',duration_ms:15000,size_bytes:1000000})`);
    await until(`document.querySelector('.feedback-saved') && document.querySelector('.state-buffering')`);
    assert.equal(await evaluate(`getComputedStyle(document.querySelector('.state-buffering')).color`), 'rgb(135, 204, 226)');
    assert.equal(await evaluate(`getComputedStyle(document.querySelector('.feedback-success-badge')).backgroundColor`), 'rgb(231, 162, 110)');
    assert.equal(await evaluate(`getComputedStyle(document.querySelector('.feedback-saved')).backgroundColor`), 'rgb(21, 22, 26)');
    await screenshot('custom-feedback', [1440]);
    await command('Page.reload');
    await until(`document.documentElement.style.getPropertyValue('--capture-color') === '#87cce2'`);
    await open();
    assert.equal(await evaluate(`document.querySelector('#theme-preset').value`), 'Ocean');
    assert.equal(await evaluate(`document.querySelector('#theme-icon').value`), 'vamp');
    // Built-in themes clear overrides, then the named preset restores colors and icon.
    await selectEmber();
    await evaluate(`document.querySelector('form').requestSubmit()`);
    await until(`document.documentElement.dataset.theme === 'ember' && !document.documentElement.style.getPropertyValue('--capture-color')`);
    await press('Escape');
    await evaluate(`window.emitController({type:'status_changed',state:'Buffering'}); window.emitController({type:'clip_saved',path:'C:/fixture/replay.mp4',duration_ms:15000,size_bytes:1000000})`);
    await until(`document.querySelector('.feedback-saved') && document.querySelector('.state-buffering')`);
    assert.equal(await evaluate(`getComputedStyle(document.querySelector('.state-buffering')).color`), 'rgb(234, 177, 124)');
    assert.equal(await evaluate(`getComputedStyle(document.querySelector('.feedback-success-badge')).backgroundColor`), 'rgb(234, 177, 124)');
    await open(); await select('#theme-preset', 'Ocean');
    await evaluate(`document.querySelector('form').requestSubmit()`);
    await until(`document.documentElement.style.getPropertyValue('--capture-color') === '#87cce2'`);
    assert.equal(await evaluate(`window.testSaves.at(-1).application.activeThemePreset`), 'Ocean');
    await input('[aria-label="Buffering and ready text"]', '#99bbcc');
    await input('#theme-preset-name', 'ocean');
    await button('Update preset');
    assert.equal(await evaluate(`document.querySelector('#theme-preset').options.length`), 2);
    await evaluate(`document.querySelector('form').requestSubmit()`);
    await until(`document.documentElement.style.getPropertyValue('--capture-color') === '#99bbcc'`);
    assert.equal(await evaluate(`window.testSaves.at(-1).application.themePresets[0].colors.capture`), '#99bbcc');
    // Deletion is staged and can be discarded without losing the persisted preset.
    await evaluate(`window.confirm = () => true`);
    await button('Delete preset');
    assert.equal(await evaluate(`document.querySelector('#theme-preset').options.length`), 1);
    await press('Escape'); await open();
    assert.equal(await evaluate(`document.querySelector('#theme-preset').value`), 'Ocean');
    await button('Delete preset');
    await evaluate(`document.querySelector('form').requestSubmit()`);
    await until(`window.testSaves.at(-1).application.themePresets.length === 0`);
    assert.equal(await evaluate(`window.testSaves.at(-1).application.activeThemePreset`), null);
    assert.equal(await evaluate(`document.documentElement.style.getPropertyValue('--capture-color')`), '#99bbcc');
    console.log("PASS: save/discard/retry; capture selection; legacy compatibility; named preset persistence and restore; independent icon; themed/custom status and real clip feedback components.");
} finally {
  socket?.close();
  edge.kill();
  vite.kill();
  await delay(300);
  await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }).catch(() => {});
}
