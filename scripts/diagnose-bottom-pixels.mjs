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

async function runDiagnostics() {
  const exePath = path.resolve("target/debug/silk.exe");
  console.log(`[DIAG] Launching: ${exePath}`);

  const evidenceDir = path.resolve("docs/evidence/diagnostics");
  fs.mkdirSync(evidenceDir, { recursive: true });

  const env = {
    ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9222",
  };

  const proc = spawn(exePath, [], { env, stdio: "ignore" });

  try {
    console.log("[DIAG] Waiting for CDP endpoint...");
    const wsUrl = await getDebuggerUrl();
    console.log(`[DIAG] Connected to CDP: ${wsUrl}`);

    const cdp = new CDPClient(wsUrl);
    await cdp.connect();
    await cdp.send("Page.enable");
    await cdp.send("DOM.enable");
    await cdp.send("CSS.enable");

    // Wait for app to settle
    await sleep(2500);

    // Get HWND
    const hwndOut = execSync(
      `pwsh -NoProfile -Command "(Get-Process -Name silk | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1).MainWindowHandle"`,
      { encoding: "utf-8" }
    ).trim();
    const hwnd = parseInt(hwndOut, 10);
    console.log(`[DIAG] Silk MainWindowHandle HWND: ${hwnd}`);

    async function inspectState(stateName) {
      console.log(`\n================================================================================`);
      console.log(`[DIAG] STATE: ${stateName}`);
      console.log(`================================================================================`);

      // 1. Evaluate DOM geometry & elements along bottom
      const domInfo = await cdp.evaluate(`
        (() => {
          const w = window.innerWidth;
          const h = window.innerHeight;
          const dpr = window.devicePixelRatio;

          // Points across bottom: y = h - 1, h - 2, h - 3, h - 5, h - 10
          const testXs = [10, Math.floor(w * 0.25), Math.floor(w * 0.5), Math.floor(w * 0.75), w - 10];
          const testYs = [h - 1, h - 2, h - 3, h - 5, h - 10];

          const pointGrid = [];
          for (const y of testYs) {
            for (const x of testXs) {
              const el = document.elementFromPoint(x, y);
              let elDesc = null;
              if (el) {
                const rect = el.getBoundingClientRect();
                const style = window.getComputedStyle(el);
                elDesc = {
                  tagName: el.tagName,
                  className: el.className,
                  id: el.id,
                  rect: { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right, width: rect.width, height: rect.height },
                  backgroundColor: style.backgroundColor,
                  backgroundImage: style.backgroundImage ? style.backgroundImage.slice(0, 60) : "none",
                  borderBottom: style.borderBottom,
                  boxShadow: style.boxShadow,
                  outline: style.outline,
                  color: style.color
                };
              }
              pointGrid.push({ x, y, element: elDesc });
            }
          }

          // Query structures
          const structures = {};
          const queryList = [
            'html', 'body', '#root', '.app-container', '.silk-titlebar',
            '.clip-modal-backdrop', '.clip-modal-container', '.clip-content-layout',
            '.clip-video-stage', '.clip-player-viewport', '.clip-video-element',
            '.clip-timeline-wrapper', '.clip-controls-bar', '.vod-review-panel',
            '.audio-mixer-drawer', '.clip-main-stage'
          ];
          for (const q of queryList) {
            const el = document.querySelector(q);
            if (el) {
              const rect = el.getBoundingClientRect();
              const s = window.getComputedStyle(el);
              structures[q] = {
                rect: { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right, width: rect.width, height: rect.height },
                backgroundColor: s.backgroundColor,
                borderBottom: s.borderBottom,
                boxShadow: s.boxShadow,
                position: s.position,
                zIndex: s.zIndex
              };
            }
          }

          return {
            window: { innerWidth: w, innerHeight: h, outerWidth: window.outerWidth, outerHeight: window.outerHeight, devicePixelRatio: dpr },
            fullscreenElement: document.fullscreenElement ? (document.fullscreenElement.className || document.fullscreenElement.tagName) : null,
            structures,
            pointGrid
          };
        })()
      `);

      // 2. Capture Win32 Screen Pixels
      const prefix = path.join(evidenceDir, `diag_${stateName}`);
      const win32Info = captureWindowBottomPixels(hwnd, prefix);
      const hierarchy = captureHierarchy(hwnd);

      // 3. Capture CDP Screenshot
      const screenshot = await cdp.send("Page.captureScreenshot", { format: "png" });
      fs.writeFileSync(`${prefix}_cdp.png`, Buffer.from(screenshot.data, "base64"));

      console.log(`[DOM Viewport] Size: ${domInfo.window.innerWidth}x${domInfo.window.innerHeight} (devicePixelRatio: ${domInfo.window.devicePixelRatio})`);
      console.log(`[DOM Fullscreen] document.fullscreenElement: ${domInfo.fullscreenElement}`);
      console.log(`[Win32 Client Area] Screen Bounds: (${win32Info.clientScreenLeft}, ${win32Info.clientScreenTop}) -> (${win32Info.clientScreenLeft + win32Info.clientScreenWidth}, ${win32Info.clientScreenBottom}) | Size: ${win32Info.clientScreenWidth}x${win32Info.clientScreenHeight}`);
      console.log(`[Win32 Window Frame] Rect: [${win32Info.winLeft}, ${win32Info.winTop}, ${win32Info.winRight}, ${win32Info.winBottom}] | Style: ${hierarchy.parentStyleHex} | ExStyle: ${hierarchy.parentExStyleHex} | IsMaximized: ${win32Info.isMaximized}`);

      console.log(`[Win32 Child Windows / WebView2 Hierarchy]:`);
      for (const child of hierarchy.children) {
        console.log(`  HWND ${child.hwnd} ("${child.className}" - "${child.title}"): Rect=[${child.left}, ${child.top}, ${child.right}, ${child.bottom}] Size=${child.width}x${child.height}, Client=${child.clientWidth}x${child.clientHeight}`);
      }

      console.log(`\n--- PIXEL ANALYSIS AT SILK WINDOW BOTTOM EDGE ---`);
      console.log(`Exact Client Bottom Row (Screen Y = ${win32Info.clientScreenBottom - 1}, yOffset = -1):`);
      console.log(`  Total scanned width: ${win32Info.totalWidthScanned}px`);
      console.log(`  White pixels (#FFFFFF / rgb 255,255,255): ${win32Info.whitePixelCountOnClientBottomRow}`);
      console.log(`  Near-white pixels (R>240, G>240, B>240): ${win32Info.nearWhitePixelCountOnClientBottomRow}`);

      console.log(`Top pixel colors on Client Bottom Row (Screen Y = ${win32Info.clientScreenBottom - 1}):`);
      const dist = Object.entries(win32Info.clientBottomRowPixelDistribution || {})
        .sort((a, b) => b[1] - a[1])
        .slice(0, 5);
      for (const [hex, count] of dist) {
        console.log(`  ${hex}: ${count}px (${((count / win32Info.totalWidthScanned) * 100).toFixed(1)}%)`);
      }

      console.log(`\nRow-by-Row Spectrum across Window Bottom Boundary:`);
      for (const s of win32Info.samples) {
        const boundaryLabel = s.yOffsetFromClientBottom < 0
          ? `[INSIDE Silk Window  (Offset ${s.yOffsetFromClientBottom}px)]`
          : `[OUTSIDE / Taskbar   (Offset +${s.yOffsetFromClientBottom}px)]`;
        console.log(`  Screen Y=${s.screenY} ${boundaryLabel}: dominant=${s.dominantColor}, white=${s.whiteCount}, nearWhite=${s.nearWhiteCount}`);
        const rowDesc = s.rowSamples.map((p) => `x=${p.x}:${p.hex}(${p.r},${p.g},${p.b})`).join(" | ");
        console.log(`    Samples: ${rowDesc}`);
      }

      console.log(`\n--- DOM ELEMENTS AT CLIENT BOTTOM EDGE (y = ${domInfo.window.innerHeight - 1}) ---`);
      const bottomElements = domInfo.pointGrid.filter((p) => p.y === domInfo.window.innerHeight - 1);
      for (const pt of bottomElements) {
        if (pt.element) {
          console.log(`  x=${pt.x}: <${pt.element.tagName.toLowerCase()} class="${pt.element.className}"> bg=${pt.element.backgroundColor}, border=${pt.element.borderBottom}, shadow=${pt.element.boxShadow}`);
        } else {
          console.log(`  x=${pt.x}: null`);
        }
      }

      return { domInfo, win32Info, hierarchy };
    }

    // Step 1: App Launched (Initial State)
    const step1 = await inspectState("1_launched");

    // Step 2: Open Clip in ClipPlayerModal
    console.log("\n[DIAG] Action: Opening clip in ClipPlayerModal...");
    await cdp.clickElement(".clip-card-thumb-container");
    await sleep(2500);

    const step2 = await inspectState("2_modal_open");

    // Step 3: Enter Fullscreen via user gesture click on symbol button
    console.log("\n[DIAG] Action: Clicking fullscreen symbol button (.clip-fullscreen-symbol-btn)...");
    const clickedFs = await cdp.clickElement(".clip-fullscreen-symbol-btn");
    console.log(`[DIAG] Fullscreen button clicked: ${clickedFs}`);
    await sleep(2500);

    const step3 = await inspectState("3_fullscreen_active");

    // Step 4: Exit Fullscreen
    console.log("\n[DIAG] Action: Exiting fullscreen...");
    await cdp.evaluate(`
      (() => {
        if (document.fullscreenElement) {
          return document.exitFullscreen();
        }
      })()
    `);
    await sleep(2500);

    const step4 = await inspectState("4_after_exit_fullscreen");

    // Step 5: Test Theater Mode
    console.log("\n[DIAG] Action: Clicking Theater Mode button...");
    const theaterBtnSelector = "button[title*='Theater Mode']";
    await cdp.clickElement(theaterBtnSelector);
    await sleep(2500);

    const step5 = await inspectState("5_theater_mode");

    // Step 6: Toggle back to Standard Mode
    console.log("\n[DIAG] Action: Clicking Standard Mode button...");
    const standardBtnSelector = "button[title*='Standard View'], button[title*='Theater']";
    await cdp.clickElement(standardBtnSelector);
    await sleep(2500);

    const step6 = await inspectState("6_back_to_standard");

    // Step 7: Close modal
    console.log("\n[DIAG] Action: Closing ClipPlayerModal...");
    await cdp.clickElement(".clip-modal-actions .button-close");
    await sleep(1500);

    const step7 = await inspectState("7_modal_closed");

    cdp.close();

    console.log("\n================================================================================");
    console.log("[DIAG] ALL DIAGNOSTIC RUNS FINISHED SUCCESSFULLY");
    console.log("================================================================================");
  } finally {
    console.log("[DIAG] Cleaning up Silk process...");
    proc.kill("SIGKILL");
    try {
      execSync("taskkill /F /IM silk.exe /T", { stdio: "ignore" });
    } catch {}
  }
}

runDiagnostics().catch((err) => {
  console.error("[DIAG ERROR]", err);
  process.exit(1);
});
