// Dependency-free Windows Edge regression test. Run: node scripts/test-settings-save-gate.mjs
// Uses isolated browser storage and a fake IPC bridge, never the installed recorder.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
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
    window.__TAURI_INTERNALS__ = {
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (command, args) => {
        if (command === 'configure_settings') {
          if (window.failSave) throw new Error('test save failure');
          window.testSaves.push(structuredClone(args.settings)); return;
        }
        if (command === 'update_theme_icon') { window.testIcons.push(args.theme); return; }
        if (command === 'plugin:event|listen') return 1;
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
  console.log("PASS: five discard paths; unsaved theme/icon isolation; failed save isolation; successful save persists.");
} finally {
  socket?.close();
  edge.kill();
  vite.kill();
  await delay(300);
  await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }).catch(() => {});
}
