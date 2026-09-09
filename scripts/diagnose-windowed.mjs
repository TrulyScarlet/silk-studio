import { spawn, execSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function getDebuggerUrl() {
  for (let i = 0; i < 40; i++) {
    try {
      const res = await fetch("http://127.0.0.1:9222/json");
      const list = await res.json();
      const page = list.find((p) => p.type === "page" && !p.url.startsWith("devtools://"));
      if (page && page.webSocketDebuggerUrl) {
        return page.webSocketDebuggerUrl;
      }
    } catch {
      // retry
    }
    await sleep(250);
  }
  throw new Error("Could not connect to CDP at http://127.0.0.1:9222/json");
}

class CDPClient {
  constructor(wsUrl) {
    this.wsUrl = wsUrl;
    this.id = 1;
    this.pending = new Map();
  }

  async connect() {
    this.ws = new WebSocket(this.wsUrl);
    await new Promise((resolve, reject) => {
      this.ws.onopen = resolve;
      this.ws.onerror = reject;
    });

    this.ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) {
          reject(new Error(msg.error.message));
        } else {
          resolve(msg.result);
        }
      }
    };
  }

  send(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = this.id++;
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  async evaluate(expression) {
    const res = await this.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
      userGesture: true,
    });
    if (res.exceptionDetails) {
      throw new Error(`Eval error: ${JSON.stringify(res.exceptionDetails)}`);
    }
    return res.result?.value;
  }

  async clickElement(selector) {
    const box = await this.evaluate(`
      (() => {
        const el = document.querySelector("${selector}");
        if (!el) return null;
        const rect = el.getBoundingClientRect();
        return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
      })()
    `);
    if (!box) return false;

    await this.send("Input.dispatchMouseEvent", {
      type: "mousePressed",
      x: box.x,
      y: box.y,
      button: "left",
      clickCount: 1,
    });
    await sleep(50);
    await this.send("Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x: box.x,
      y: box.y,
      button: "left",
      clickCount: 1,
    });
    return true;
  }

  close() {
    if (this.ws) {
      this.ws.close();
    }
  }
}

function captureWindowBottomPixels(hwnd, outputPrefix) {
  const scriptPath = path.resolve("scripts/capture-bottom-pixels.ps1");
  const normalizedPrefix = outputPrefix.replace(/\\/g, "/");
  const cmd = `pwsh -NoProfile -ExecutionPolicy Bypass -File "${scriptPath}" -Hwnd ${hwnd} -OutputPrefix "${normalizedPrefix}"`;
  const out = execSync(cmd, { encoding: "utf-8", maxBuffer: 10 * 1024 * 1024 });
  return JSON.parse(out);
}

function captureHierarchy(hwnd) {
  const scriptPath = path.resolve("scripts/capture-hierarchy.ps1");
  const cmd = `pwsh -NoProfile -ExecutionPolicy Bypass -File "${scriptPath}" -Hwnd ${hwnd} -OutputPrefix "dummy"`;
  const out = execSync(cmd, { encoding: "utf-8", maxBuffer: 10 * 1024 * 1024 });
  return JSON.parse(out);
}

async function runWindowedTest() {
  const exePath = path.resolve("target/debug/silk.exe");
  console.log(`\n================================================================================`);
  console.log(`[DIAG] TESTING RESTORED / UNMAXIMIZED WINDOW MODE`);
  console.log(`================================================================================`);

  const evidenceDir = path.resolve("docs/evidence/diagnostics");
  fs.mkdirSync(evidenceDir, { recursive: true });

  const env = {
    ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9222",
  };

  const proc = spawn(exePath, [], { env, stdio: "ignore" });

  try {
    const wsUrl = await getDebuggerUrl();
    const cdp = new CDPClient(wsUrl);
    await cdp.connect();
    await cdp.send("Page.enable");
    await cdp.send("DOM.enable");
    await sleep(2500);

    const hwndOut = execSync(
      `pwsh -NoProfile -Command "(Get-Process -Name silk | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1).MainWindowHandle"`,
      { encoding: "utf-8" }
    ).trim();
    const hwnd = parseInt(hwndOut, 10);

    // Unmaximize window via titlebar button
    console.log("[DIAG] Unmaximizing window...");
    await cdp.clickElement(".titlebar-btn-maximize");
    await sleep(1500);

    // Inspect windowed state
    const win32Windowed = captureWindowBottomPixels(hwnd, path.join(evidenceDir, "diag_windowed_1_initial"));
    console.log(`[Win32 Windowed] Rect: [${win32Windowed.winLeft}, ${win32Windowed.winTop}, ${win32Windowed.winRight}, ${win32Windowed.winBottom}] Size: ${win32Windowed.winWidth}x${win32Windowed.winHeight} | IsMaximized: ${win32Windowed.isMaximized}`);
    console.log(`  Client bottom row (Screen Y=${win32Windowed.clientScreenBottom - 1}): white=${win32Windowed.whitePixelCountOnClientBottomRow}, nearWhite=${win32Windowed.nearWhitePixelCountOnClientBottomRow}`);

    // Open clip
    console.log("[DIAG] Opening clip in windowed mode...");
    await cdp.clickElement(".clip-card-thumb-container");
    await sleep(2000);

    const win32Modal = captureWindowBottomPixels(hwnd, path.join(evidenceDir, "diag_windowed_2_modal"));
    console.log(`[Win32 Modal] Rect: [${win32Modal.winLeft}, ${win32Modal.winTop}, ${win32Modal.winRight}, ${win32Modal.winBottom}] Size: ${win32Modal.winWidth}x${win32Modal.winHeight}`);
    console.log(`  Client bottom row (Screen Y=${win32Modal.clientScreenBottom - 1}): white=${win32Modal.whitePixelCountOnClientBottomRow}`);

    // Enter fullscreen
    console.log("[DIAG] Entering fullscreen from windowed mode...");
    await cdp.clickElement(".clip-fullscreen-symbol-btn");
    await sleep(2500);

    const win32Fullscreen = captureWindowBottomPixels(hwnd, path.join(evidenceDir, "diag_windowed_3_fullscreen"));
    const hierFs = captureHierarchy(hwnd);
    console.log(`[Win32 Fullscreen] Rect: [${win32Fullscreen.winLeft}, ${win32Fullscreen.winTop}, ${win32Fullscreen.winRight}, ${win32Fullscreen.winBottom}] Size: ${win32Fullscreen.winWidth}x${win32Fullscreen.winHeight}`);
    console.log(`  Child WRY_WEBVIEW: ${JSON.stringify(hierFs.children.find(c => c.className === "WRY_WEBVIEW"))}`);

    // Exit fullscreen
    console.log("[DIAG] Exiting fullscreen back to windowed mode...");
    await cdp.evaluate(`(() => { if (document.fullscreenElement) return document.exitFullscreen(); })()`);
    await sleep(2500);

    const win32AfterFs = captureWindowBottomPixels(hwnd, path.join(evidenceDir, "diag_windowed_4_after_fullscreen"));
    const hierAfterFs = captureHierarchy(hwnd);
    console.log(`[Win32 After Fullscreen] Rect: [${win32AfterFs.winLeft}, ${win32AfterFs.winTop}, ${win32AfterFs.winRight}, ${win32AfterFs.winBottom}] Size: ${win32AfterFs.winWidth}x${win32AfterFs.winHeight} | IsMaximized: ${win32AfterFs.isMaximized}`);
    console.log(`  Child WRY_WEBVIEW: ${JSON.stringify(hierAfterFs.children.find(c => c.className === "WRY_WEBVIEW"))}`);
    console.log(`  Client bottom row: white=${win32AfterFs.whitePixelCountOnClientBottomRow}, nearWhite=${win32AfterFs.nearWhitePixelCountOnClientBottomRow}`);

    cdp.close();
  } finally {
    proc.kill("SIGKILL");
    try {
      execSync("taskkill /F /IM silk.exe /T", { stdio: "ignore" });
    } catch {}
  }
}

runWindowedTest().catch((err) => {
  console.error("[DIAG ERROR]", err);
  process.exit(1);
});
