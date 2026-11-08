// Dependency-free Windows Edge regression and screenshots. Run: node scripts/check-app-updates.mjs
// Uses isolated browser storage and a fake IPC bridge, never the installed recorder.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm, readFile, writeFile } from "node:fs/promises";
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
    const timeout = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 30_000);
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

  for (const [width,height] of [[1280,800],[1440,900],[1920,1080]]) {
    await command('Page.navigate',{url:'about:blank'});
    await until('location.href === "about:blank"');
    await command('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
    await command('Page.navigate',{url:'http://127.0.0.1:5174/?preview=1'});
    await until('document.querySelectorAll(".clip-card").length === 5');
    const result = await evaluate(await readFile('scripts/test-app-updates.js','utf8'));
    console.log(JSON.stringify({passed:result.passed,viewport:{width,height},installs:result.installs}));
    await delay(300);
    const shot=await command('Page.captureScreenshot',{format:'png'});
    await writeFile('target/app-updates-'+width+'.png',Buffer.from(shot.data,'base64'));
  }
} finally {
  socket?.close(); edge.kill(); vite.kill();
  await delay(1000);
  await rm(profile,{recursive:true,force:true,maxRetries:5,retryDelay:250});
}
