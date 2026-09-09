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

function queryWin32Rects(hwnd, action = "Query") {
  const scriptPath = path.resolve("scripts/query-win32-rects.ps1");
  const cmd = `pwsh -NoProfile -ExecutionPolicy Bypass -File "${scriptPath}" -Hwnd ${hwnd} -Action "${action}"`;
  const out = execSync(cmd, { encoding: "utf-8", maxBuffer: 10 * 1024 * 1024 });
  return JSON.parse(out);
}

function printReport(stageName, rep, domInfo = null) {
  console.log(`\n--------------------------------------------------------------------------------`);
  console.log(`[STAGE] ${stageName}`);
  console.log(`--------------------------------------------------------------------------------`);
  console.log(`  Action Executed    : ${rep.actionExecuted}`);
  console.log(`  Screen Resolution  : ${rep.screenWidth} x ${rep.screenHeight}`);
  console.log(`  WorkArea           : [Left=${rep.workAreaLeft}, Top=${rep.workAreaTop}, Right=${rep.workAreaRight}, Bottom=${rep.workAreaBottom}] (${rep.workAreaWidth} x ${rep.workAreaHeight})`);
  console.log(`  Window Rect (Win32): [Left=${rep.winLeft}, Top=${rep.winTop}, Right=${rep.winRight}, Bottom=${rep.winBottom}] (${rep.winWidth} x ${rep.winHeight})`);
  console.log(`  Client Rect (Win32): [Left=${rep.clientLeft}, Top=${rep.clientTop}, Right=${rep.clientRight}, Bottom=${rep.clientBottom}] (${rep.clientWidth} x ${rep.clientHeight})`);
  console.log(`  Client Screen Pos  : [Left=${rep.clientScreenLeft}, Top=${rep.clientScreenTop}, Right=${rep.clientScreenRight}, Bottom=${rep.clientScreenBottom}]`);
  console.log(`  DWM Extended Bounds: [Left=${rep.dwmLeft}, Top=${rep.dwmTop}, Right=${rep.dwmRight}, Bottom=${rep.dwmBottom}]`);
  console.log(`  Maximized (IsZoomed): ${rep.isMaximized} | Minimized (IsIconic): ${rep.isIconic}`);
  console.log(`  Parent Styles      : Style=${rep.styleHex}, ExStyle=${rep.exStyleHex}`);

  if (rep.children && rep.children.length > 0) {
    console.log(`  Child Windows:`);
    for (const ch of rep.children) {
      console.log(`    - HWND ${ch.hwnd} "${ch.className}": Rect=[${ch.winLeft}, ${ch.winTop}, ${ch.winRight}, ${ch.winBottom}] (${ch.winWidth}x${ch.winHeight}), Client=${ch.clientWidth}x${ch.clientHeight}`);
    }
  }

  if (domInfo) {
    console.log(`  DOM Viewport       : innerWidth=${domInfo.innerWidth}, innerHeight=${domInfo.innerHeight}, outerWidth=${domInfo.outerWidth}, outerHeight=${domInfo.outerHeight}, dpr=${domInfo.devicePixelRatio}`);
    console.log(`  DOM FullscreenElem : ${domInfo.fullscreenElement}`);
  }

  console.log(`  >>> Bottom Gap to WorkArea (ClientScreenBottom - WorkAreaBottom): ${rep.bottomGapToWorkArea} px`);
  console.log(`  >>> Bottom Gap to Screen   (ClientScreenBottom - ScreenHeight)  : ${rep.bottomGapToScreen} px`);
  console.log(`  >>> Top Gap to WorkArea    (ClientScreenTop - WorkAreaTop)      : ${rep.topGapToWorkArea} px`);
  console.log(`  >>> Left Gap to WorkArea   (ClientScreenLeft - WorkAreaLeft)    : ${rep.leftGapToWorkArea} px`);
  console.log(`  >>> Right Gap to WorkArea  (ClientScreenRight - WorkAreaRight)  : ${rep.rightGapToWorkArea} px`);
}

async function getDomInfo(cdp) {
  return await cdp.evaluate(`
    (() => {
      return {
        innerWidth: window.innerWidth,
        innerHeight: window.innerHeight,
        outerWidth: window.outerWidth,
        outerHeight: window.outerHeight,
        devicePixelRatio: window.devicePixelRatio,
        fullscreenElement: document.fullscreenElement ? (document.fullscreenElement.className || document.fullscreenElement.tagName) : null
      };
    })()
  `);
}

async function runComprehensiveTest() {
  console.log("================================================================================");
  console.log(" [SILK FULLSCREEN RECT, WORKAREA GAP & WIN32 RESTORATION TEST]");
  console.log("================================================================================");

  try {
    execSync("taskkill /F /IM silk.exe /T", { stdio: "ignore" });
  } catch {}

  const exePath = path.resolve("target/debug/silk.exe");
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
    console.log(`[TEST] Silk MainWindowHandle HWND: ${hwnd} (0x${hwnd.toString(16)})`);

    // ========================================================================
    // SCENARIO 1: Silk Launched Maximized -> Enter Fullscreen -> Exit Fullscreen
    // ========================================================================
    console.log("\n================================================================================");
    console.log(" [SCENARIO 1: MAXIMIZED LAUNCH -> FULLSCREEN -> EXIT FULLSCREEN]");
    console.log("================================================================================");

    const s1_before = queryWin32Rects(hwnd, "Query");
    const s1_dom_before = await getDomInfo(cdp);
    printReport("1.1 BEFORE FULLSCREEN (Maximized Launch)", s1_before, s1_dom_before);

    // Open clip modal
    await cdp.clickElement(".clip-card-thumb-container");
    await sleep(1500);

    // Enter Fullscreen
    await cdp.clickElement(".clip-fullscreen-symbol-btn");
    await sleep(2500);
    const s1_inFs = queryWin32Rects(hwnd, "Query");
    const s1_dom_inFs = await getDomInfo(cdp);
    printReport("1.2 IN FULLSCREEN", s1_inFs, s1_dom_inFs);

    // Exit Fullscreen
    await cdp.evaluate(`(async () => {
      if (window.__TAURI__?.core) {
        await window.__TAURI__.core.invoke("app_window_set_fullscreen", { fullscreen: false });
      }
    })()`);
    await sleep(2500);
    const s1_afterFs = queryWin32Rects(hwnd, "Query");
    const s1_dom_afterFs = await getDomInfo(cdp);
    printReport("1.3 AFTER EXITING FULLSCREEN", s1_afterFs, s1_dom_afterFs);

    // ========================================================================
    // SCENARIO 2: Silk Windowed/Restored Mode -> Fullscreen -> Exit Fullscreen
    // ========================================================================
    console.log("\n================================================================================");
    console.log(" [SCENARIO 2: RESTORED/WINDOWED MODE -> FULLSCREEN -> EXIT FULLSCREEN]");
    console.log("================================================================================");

    // Toggle unmaximize from titlebar
    await cdp.clickElement(".titlebar-btn-maximize");
    await sleep(1500);

    const s2_windowed = queryWin32Rects(hwnd, "Query");
    const s2_dom_windowed = await getDomInfo(cdp);
    printReport("2.1 BEFORE FULLSCREEN (Restored/Windowed Mode)", s2_windowed, s2_dom_windowed);

    // Enter Fullscreen from windowed
    await cdp.clickElement(".clip-fullscreen-symbol-btn");
    await sleep(2500);
    const s2_inFs = queryWin32Rects(hwnd, "Query");
    const s2_dom_inFs = await getDomInfo(cdp);
    printReport("2.2 IN FULLSCREEN (from windowed)", s2_inFs, s2_dom_inFs);

    // Exit Fullscreen
    await cdp.evaluate(`(async () => {
      if (window.__TAURI__?.core) {
        await window.__TAURI__.core.invoke("app_window_set_fullscreen", { fullscreen: false });
      }
    })()`);
    await sleep(2500);
    const s2_afterFs = queryWin32Rects(hwnd, "Query");
    const s2_dom_afterFs = await getDomInfo(cdp);
    printReport("2.3 AFTER EXITING FULLSCREEN (back to windowed)", s2_afterFs, s2_dom_afterFs);

    // Maximize back from windowed
    await cdp.clickElement(".titlebar-btn-maximize");
    await sleep(1500);
    const s2_maximizedBack = queryWin32Rects(hwnd, "Query");
    printReport("2.4 RE-MAXIMIZED VIA TITLEBAR BUTTON", s2_maximizedBack);

    // ========================================================================
    // SCENARIO 3: Deep Win32 API Call Sequence Analysis
    // ========================================================================
    console.log("\n================================================================================");
    console.log(" [SCENARIO 3: TESTING EXACT WIN32 API SEQUENCES TO SNAP TO 0PX GAP]");
    console.log("================================================================================");

    const apiTests = [
      {
        id: "WIN32_01_SW_MAXIMIZE",
        desc: "ShowWindow(hWnd, SW_MAXIMIZE)",
        action: "ShowWindow_SW_MAXIMIZE",
      },
      {
        id: "WIN32_02_SWP_FRAMECHANGED",
        desc: "SetWindowPos(SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE)",
        action: "SetWindowPos_SWP_FRAMECHANGED",
      },
      {
        id: "WIN32_03_SW_RESTORE_MAXIMIZE",
        desc: "ShowWindow(SW_RESTORE) -> ShowWindow(SW_MAXIMIZE)",
        action: "Restore_Then_Maximize",
      },
      {
        id: "WIN32_04_SWP_WORKAREA",
        desc: "SetWindowPos(WorkArea X, Y, W, H, SWP_FRAMECHANGED)",
        action: "SetWindowPos_WorkArea",
      },
      {
        id: "WIN32_05_WM_SYSCOMMAND_MAXIMIZE",
        desc: "SendMessage(WM_SYSCOMMAND, SC_MAXIMIZE, 0)",
        action: "WmSysCommand_SC_MAXIMIZE",
      },
      {
        id: "WIN32_06_WM_SYSCOMMAND_RESTORE_MAXIMIZE",
        desc: "SendMessage(WM_SYSCOMMAND, SC_RESTORE) -> SC_MAXIMIZE",
        action: "WmSysCommand_Restore_Then_Maximize",
      },
      {
        id: "WIN32_07_RESTORE_STYLES_AND_SETWORKAREA",
        desc: "SetWindowLong(GWL_STYLE, 0x15CF0000) -> SetWindowPos(WorkArea Frame)",
        action: "Restore_Style_And_SetWorkArea",
      },
      {
        id: "WIN32_08_RESTORE_STYLES_AND_MAXIMIZE",
        desc: "SetWindowLong(GWL_STYLE, 0x15CF0000) -> ShowWindow(SW_MAXIMIZE)",
        action: "Restore_Style_And_Maximize",
      }
    ];

    const results = [];

    for (const t of apiTests) {
      // Re-enter and exit fullscreen
      await cdp.evaluate(`(async () => {
        if (window.__TAURI__?.core) {
          await window.__TAURI__.core.invoke("app_window_set_fullscreen", { fullscreen: true });
        }
      })()`);
      await sleep(1500);
      await cdp.evaluate(`(async () => {
        if (window.__TAURI__?.core) {
          await window.__TAURI__.core.invoke("app_window_set_fullscreen", { fullscreen: false });
        }
      })()`);
      await sleep(1500);

      const pre = queryWin32Rects(hwnd, "Query");
      queryWin32Rects(hwnd, t.action);
      await sleep(400);
      const post = queryWin32Rects(hwnd, "Query");
      const postDom = await getDomInfo(cdp);

      printReport(`RESULT: ${t.desc}`, post, postDom);

      results.push({
        id: t.id,
        description: t.desc,
        action: t.action,
        styleBefore: pre.styleHex,
        styleAfter: post.styleHex,
        winRectBefore: `[${pre.winLeft},${pre.winTop},${pre.winRight},${pre.winBottom}]`,
        winRectAfter: `[${post.winLeft},${post.winTop},${post.winRight},${post.winBottom}]`,
        clientBottom: post.clientScreenBottom,
        workAreaBottom: post.workAreaBottom,
        gapToWorkArea: post.bottomGapToWorkArea,
        dwmBounds: `[${post.dwmLeft},${post.dwmTop},${post.dwmRight},${post.dwmBottom}]`,
        isMaximized: post.isMaximized,
        gapZero: post.bottomGapToWorkArea === 0,
        frameSnaped: post.winBottom <= post.workAreaBottom + 8,
      });
    }

    console.log("\n================================================================================");
    console.log(" [FULL TEST MATRIX RESULTS]");
    console.log("================================================================================");
    console.table(results);

    cdp.close();
  } finally {
    proc.kill("SIGKILL");
    try {
      execSync("taskkill /F /IM silk.exe /T", { stdio: "ignore" });
    } catch {}
  }
}

runComprehensiveTest().catch((err) => {
  console.error("[TEST ERROR]", err);
  process.exit(1);
});
