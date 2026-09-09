param(
    [Parameter(Mandatory=$false)]
    [int64]$Hwnd = 0,

    [Parameter(Mandatory=$false)]
    [string]$Action = "Query"
)

$ErrorActionPreference = "Stop"

$code = @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace SilkWin32Diag {
    public class RectTester {
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool ClientToScreen(IntPtr hWnd, ref POINT lpPoint);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool SystemParametersInfo(uint uiAction, uint uiParam, out RECT pvParam, uint fWinIni);

        [DllImport("user32.dll")]
        public static extern int GetSystemMetrics(int nIndex);

        [DllImport("user32.dll")]
        public static extern bool IsZoomed(IntPtr hWnd);

        [DllImport("user32.dll")]
        public static extern bool IsIconic(IntPtr hWnd);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern int GetWindowLong(IntPtr hWnd, int nIndex);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern int SetWindowLong(IntPtr hWnd, int nIndex, int dwNewLong);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern IntPtr SendMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);

        [DllImport("dwmapi.dll")]
        public static extern int DwmGetWindowAttribute(IntPtr hwnd, int dwAttribute, out RECT pvAttribute, int cbAttribute);

        [DllImport("user32.dll")]
        public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);

        [DllImport("user32.dll")]
        public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

        [DllImport("user32.dll")]
        public static extern bool EnumChildWindows(IntPtr hWndParent, EnumWindowsProc lpEnumFunc, IntPtr lParam);

        public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

        public const uint SPI_GETWORKAREA = 0x0030;
        public const int SM_CXSCREEN = 0;
        public const int SM_CYSCREEN = 1;
        public const int GWL_STYLE = -16;
        public const int GWL_EXSTYLE = -20;
        public const int DWMWA_EXTENDED_FRAME_BOUNDS = 9;

        // Styles
        public const int WS_VISIBLE = 0x10000000;
        public const int WS_CLIPSIBLINGS = 0x04000000;
        public const int WS_CLIPCHILDREN = 0x02000000;
        public const int WS_MAXIMIZE = 0x01000000;
        public const int WS_CAPTION = 0x00C00000;
        public const int WS_BORDER = 0x00800000;
        public const int WS_DLGFRAME = 0x00400000;
        public const int WS_SYSMENU = 0x00080000;
        public const int WS_THICKFRAME = 0x00040000;
        public const int WS_MINIMIZEBOX = 0x00020000;
        public const int WS_MAXIMIZEBOX = 0x00010000;
        public const int WS_POPUP = unchecked((int)0x80000000);

        // ShowWindow commands
        public const int SW_HIDE = 0;
        public const int SW_SHOWNORMAL = 1;
        public const int SW_SHOWMINIMIZED = 2;
        public const int SW_MAXIMIZE = 3;
        public const int SW_SHOWNOACTIVATE = 4;
        public const int SW_SHOW = 5;
        public const int SW_MINIMIZE = 6;
        public const int SW_SHOWMINNOACTIVE = 7;
        public const int SW_SHOWNA = 8;
        public const int SW_RESTORE = 9;

        // SetWindowPos flags
        public const uint SWP_NOSIZE = 0x0001;
        public const uint SWP_NOMOVE = 0x0002;
        public const uint SWP_NOZORDER = 0x0004;
        public const uint SWP_NOREDRAW = 0x0008;
        public const uint SWP_NOACTIVATE = 0x0010;
        public const uint SWP_FRAMECHANGED = 0x0020;
        public const uint SWP_SHOWWINDOW = 0x0040;
        public const uint SWP_HIDEWINDOW = 0x0080;
        public const uint SWP_NOCOPYBITS = 0x0100;
        public const uint SWP_NOOWNERZORDER = 0x0200;
        public const uint SWP_NOSENDCHANGING = 0x0400;

        // Window Messages
        public const uint WM_SYSCOMMAND = 0x0112;
        public const uint SC_MAXIMIZE = 0xF030;
        public const uint SC_RESTORE = 0xF120;

        [StructLayout(LayoutKind.Sequential)]
        public struct RECT {
            public int Left;
            public int Top;
            public int Right;
            public int Bottom;
        }

        [StructLayout(LayoutKind.Sequential)]
        public struct POINT {
            public int X;
            public int Y;
        }

        public class ChildWindowInfo {
            public long hwnd;
            public string className;
            public string title;
            public int winLeft;
            public int winTop;
            public int winRight;
            public int winBottom;
            public int winWidth;
            public int winHeight;
            public int clientWidth;
            public int clientHeight;
            public string styleHex;
            public string exStyleHex;
        }

        public class RectReport {
            public string actionExecuted;
            public long hwnd;
            public int screenWidth;
            public int screenHeight;

            public int workAreaLeft;
            public int workAreaTop;
            public int workAreaRight;
            public int workAreaBottom;
            public int workAreaWidth;
            public int workAreaHeight;

            public int winLeft;
            public int winTop;
            public int winRight;
            public int winBottom;
            public int winWidth;
            public int winHeight;

            public int clientLeft;
            public int clientTop;
            public int clientRight;
            public int clientBottom;
            public int clientWidth;
            public int clientHeight;

            public int clientScreenLeft;
            public int clientScreenTop;
            public int clientScreenRight;
            public int clientScreenBottom;

            public int dwmLeft;
            public int dwmTop;
            public int dwmRight;
            public int dwmBottom;

            public bool isMaximized;
            public bool isIconic;
            public string styleHex;
            public string exStyleHex;

            public int bottomGapToWorkArea; // clientScreenBottom - workAreaBottom
            public int bottomGapToScreen;   // clientScreenBottom - screenHeight
            public int topGapToWorkArea;    // clientScreenTop - workAreaTop
            public int leftGapToWorkArea;   // clientScreenLeft - workAreaLeft
            public int rightGapToWorkArea;  // clientScreenRight - workAreaRight

            public List<ChildWindowInfo> children = new List<ChildWindowInfo>();
        }

        public static RectReport ExecuteActionAndQuery(long hwndLong, string action) {
            IntPtr hWnd = new IntPtr(hwndLong);

            if (hWnd != IntPtr.Zero) {
                switch (action) {
                    case "ShowWindow_SW_MAXIMIZE":
                        ShowWindow(hWnd, SW_MAXIMIZE);
                        break;
                    case "ShowWindow_SW_RESTORE":
                        ShowWindow(hWnd, SW_RESTORE);
                        break;
                    case "Restore_Then_Maximize":
                        ShowWindow(hWnd, SW_RESTORE);
                        System.Threading.Thread.Sleep(50);
                        ShowWindow(hWnd, SW_MAXIMIZE);
                        break;
                    case "SetWindowPos_SWP_FRAMECHANGED":
                        SetWindowPos(hWnd, IntPtr.Zero, 0, 0, 0, 0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                        break;
                    case "SetWindowPos_WorkArea":
                        RECT waSnap;
                        SystemParametersInfo(SPI_GETWORKAREA, 0, out waSnap, 0);
                        SetWindowPos(hWnd, IntPtr.Zero, waSnap.Left, waSnap.Top,
                            waSnap.Right - waSnap.Left, waSnap.Bottom - waSnap.Top,
                            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                        break;
                    case "WmSysCommand_SC_MAXIMIZE":
                        SendMessage(hWnd, WM_SYSCOMMAND, (IntPtr)SC_MAXIMIZE, IntPtr.Zero);
                        break;
                    case "WmSysCommand_Restore_Then_Maximize":
                        SendMessage(hWnd, WM_SYSCOMMAND, (IntPtr)SC_RESTORE, IntPtr.Zero);
                        System.Threading.Thread.Sleep(50);
                        SendMessage(hWnd, WM_SYSCOMMAND, (IntPtr)SC_MAXIMIZE, IntPtr.Zero);
                        break;
                    case "FrameChanged_Then_Maximize":
                        SetWindowPos(hWnd, IntPtr.Zero, 0, 0, 0, 0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                        System.Threading.Thread.Sleep(50);
                        ShowWindow(hWnd, SW_MAXIMIZE);
                        break;
                    case "Restore_Style_And_Maximize":
                        // Restores 0x15CF0000 style flags (WS_THICKFRAME | WS_CAPTION | WS_SYSMENU | WS_MAXIMIZEBOX | WS_MINIMIZEBOX)
                        int curStyle = GetWindowLong(hWnd, GWL_STYLE);
                        int targetStyle = curStyle | WS_THICKFRAME | WS_CAPTION | WS_SYSMENU | WS_MAXIMIZEBOX | WS_MINIMIZEBOX;
                        SetWindowLong(hWnd, GWL_STYLE, targetStyle);
                        SetWindowPos(hWnd, IntPtr.Zero, 0, 0, 0, 0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                        System.Threading.Thread.Sleep(50);
                        ShowWindow(hWnd, SW_MAXIMIZE);
                        break;
                    case "Restore_Style_And_SetWorkArea":
                        int s = GetWindowLong(hWnd, GWL_STYLE);
                        SetWindowLong(hWnd, GWL_STYLE, s | WS_THICKFRAME | WS_CAPTION | WS_SYSMENU | WS_MAXIMIZEBOX | WS_MINIMIZEBOX);
                        RECT wa2;
                        SystemParametersInfo(SPI_GETWORKAREA, 0, out wa2, 0);
                        // For a maximized window with standard sizing frame (-8, -8), window bounds are [-8, -8, 2568, 1400]
                        SetWindowPos(hWnd, IntPtr.Zero, wa2.Left - 8, wa2.Top - 8,
                            (wa2.Right - wa2.Left) + 16, (wa2.Bottom - wa2.Top) + 16,
                            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                        break;
                    case "Query":
                    default:
                        break;
                }
            }

            // Query current metrics
            RECT wa;
            SystemParametersInfo(SPI_GETWORKAREA, 0, out wa, 0);

            int screenW = GetSystemMetrics(SM_CXSCREEN);
            int screenH = GetSystemMetrics(SM_CYSCREEN);

            RectReport rep = new RectReport();
            rep.actionExecuted = action;
            rep.hwnd = hwndLong;
            rep.screenWidth = screenW;
            rep.screenHeight = screenH;

            rep.workAreaLeft = wa.Left;
            rep.workAreaTop = wa.Top;
            rep.workAreaRight = wa.Right;
            rep.workAreaBottom = wa.Bottom;
            rep.workAreaWidth = wa.Right - wa.Left;
            rep.workAreaHeight = wa.Bottom - wa.Top;

            if (hWnd != IntPtr.Zero) {
                RECT wRect;
                GetWindowRect(hWnd, out wRect);
                RECT cRect;
                GetClientRect(hWnd, out cRect);

                POINT pt = new POINT { X = 0, Y = 0 };
                ClientToScreen(hWnd, ref pt);

                RECT dwmRect = new RECT();
                DwmGetWindowAttribute(hWnd, DWMWA_EXTENDED_FRAME_BOUNDS, out dwmRect, Marshal.SizeOf(typeof(RECT)));

                rep.winLeft = wRect.Left;
                rep.winTop = wRect.Top;
                rep.winRight = wRect.Right;
                rep.winBottom = wRect.Bottom;
                rep.winWidth = wRect.Right - wRect.Left;
                rep.winHeight = wRect.Bottom - wRect.Top;

                rep.clientLeft = cRect.Left;
                rep.clientTop = cRect.Top;
                rep.clientRight = cRect.Right;
                rep.clientBottom = cRect.Bottom;
                rep.clientWidth = cRect.Right - cRect.Left;
                rep.clientHeight = cRect.Bottom - cRect.Top;

                rep.clientScreenLeft = pt.X;
                rep.clientScreenTop = pt.Y;
                rep.clientScreenRight = pt.X + rep.clientWidth;
                rep.clientScreenBottom = pt.Y + rep.clientHeight;

                rep.dwmLeft = dwmRect.Left;
                rep.dwmTop = dwmRect.Top;
                rep.dwmRight = dwmRect.Right;
                rep.dwmBottom = dwmRect.Bottom;

                rep.isMaximized = IsZoomed(hWnd);
                rep.isIconic = IsIconic(hWnd);
                rep.styleHex = string.Format("0x{0:X8}", GetWindowLong(hWnd, GWL_STYLE));
                rep.exStyleHex = string.Format("0x{0:X8}", GetWindowLong(hWnd, GWL_EXSTYLE));

                rep.bottomGapToWorkArea = rep.clientScreenBottom - rep.workAreaBottom;
                rep.bottomGapToScreen = rep.clientScreenBottom - rep.screenHeight;
                rep.topGapToWorkArea = rep.clientScreenTop - rep.workAreaTop;
                rep.leftGapToWorkArea = rep.clientScreenLeft - rep.workAreaLeft;
                rep.rightGapToWorkArea = rep.clientScreenRight - rep.workAreaRight;

                EnumChildWindows(hWnd, (childHwnd, lParam) => {
                    StringBuilder classSb = new StringBuilder(256);
                    GetClassName(childHwnd, classSb, 256);
                    StringBuilder textSb = new StringBuilder(256);
                    GetWindowText(childHwnd, textSb, 256);

                    RECT cwRect;
                    GetWindowRect(childHwnd, out cwRect);
                    RECT ccRect;
                    GetClientRect(childHwnd, out ccRect);

                    ChildWindowInfo ci = new ChildWindowInfo {
                        hwnd = childHwnd.ToInt64(),
                        className = classSb.ToString(),
                        title = textSb.ToString(),
                        winLeft = cwRect.Left,
                        winTop = cwRect.Top,
                        winRight = cwRect.Right,
                        winBottom = cwRect.Bottom,
                        winWidth = cwRect.Right - cwRect.Left,
                        winHeight = cwRect.Bottom - cwRect.Top,
                        clientWidth = ccRect.Right - ccRect.Left,
                        clientHeight = ccRect.Bottom - ccRect.Top,
                        styleHex = string.Format("0x{0:X8}", GetWindowLong(childHwnd, GWL_STYLE)),
                        exStyleHex = string.Format("0x{0:X8}", GetWindowLong(childHwnd, GWL_EXSTYLE))
                    };
                    rep.children.Add(ci);
                    return true;
                }, IntPtr.Zero);
            }

            return rep;
        }
    }
}
"@

if (-not ([System.Management.Automation.PSTypeName]'SilkWin32Diag.RectTester').Type) {
    Add-Type -TypeDefinition $code
}

$report = [SilkWin32Diag.RectTester]::ExecuteActionAndQuery($Hwnd, $Action)
$report | ConvertTo-Json -Depth 6
