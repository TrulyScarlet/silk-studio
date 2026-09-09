param(
    [Parameter(Mandatory=$true)]
    [int64]$Hwnd,

    [Parameter(Mandatory=$true)]
    [string]$OutputPrefix
)

$ErrorActionPreference = "Stop"

$code = @"
using System;
using System.IO;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace SilkDiag {
    public class Win32HierarchyDiag {
        [DllImport("user32.dll")]
        public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

        [DllImport("user32.dll")]
        public static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);

        [DllImport("user32.dll")]
        public static extern bool ClientToScreen(IntPtr hWnd, ref POINT lpPoint);

        [DllImport("user32.dll")]
        public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);

        [DllImport("user32.dll")]
        public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

        [DllImport("user32.dll")]
        public static extern bool EnumChildWindows(IntPtr hWndParent, EnumWindowsProc lpEnumFunc, IntPtr lParam);

        [DllImport("user32.dll")]
        public static extern int GetWindowLong(IntPtr hWnd, int nIndex);

        [DllImport("user32.dll")]
        public static extern IntPtr GetDC(IntPtr hWnd);

        [DllImport("user32.dll")]
        public static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);

        [DllImport("gdi32.dll")]
        public static extern uint GetPixel(IntPtr hDC, int XPos, int YPos);

        [DllImport("dwmapi.dll")]
        public static extern int DwmGetWindowAttribute(IntPtr hwnd, int dwAttribute, out RECT pvAttribute, int cbAttribute);

        public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

        public const int GWL_STYLE = -16;
        public const int GWL_EXSTYLE = -20;

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

        public class ChildInfo {
            public long hwnd;
            public string className;
            public string title;
            public int left;
            public int top;
            public int right;
            public int bottom;
            public int width;
            public int height;
            public int clientWidth;
            public int clientHeight;
            public string styleHex;
            public string exStyleHex;
        }

        public class HierarchyResult {
            public long parentHwnd;
            public int parentLeft;
            public int parentTop;
            public int parentRight;
            public int parentBottom;
            public int parentWidth;
            public int parentHeight;
            public int parentClientWidth;
            public int parentClientHeight;
            public string parentStyleHex;
            public string parentExStyleHex;
            public List<ChildInfo> children = new List<ChildInfo>();
        }

        public static HierarchyResult Inspect(long hwndLong) {
            IntPtr hWnd = new IntPtr(hwndLong);
            HierarchyResult res = new HierarchyResult();
            res.parentHwnd = hwndLong;

            RECT rect;
            GetWindowRect(hWnd, out rect);
            RECT clientRect;
            GetClientRect(hWnd, out clientRect);

            res.parentLeft = rect.Left;
            res.parentTop = rect.Top;
            res.parentRight = rect.Right;
            res.parentBottom = rect.Bottom;
            res.parentWidth = rect.Right - rect.Left;
            res.parentHeight = rect.Bottom - rect.Top;
            res.parentClientWidth = clientRect.Right - clientRect.Left;
            res.parentClientHeight = clientRect.Bottom - clientRect.Top;
            res.parentStyleHex = string.Format("0x{0:X8}", GetWindowLong(hWnd, GWL_STYLE));
            res.parentExStyleHex = string.Format("0x{0:X8}", GetWindowLong(hWnd, GWL_EXSTYLE));

            EnumChildWindows(hWnd, (childHwnd, lParam) => {
                StringBuilder classSb = new StringBuilder(256);
                GetClassName(childHwnd, classSb, 256);

                StringBuilder textSb = new StringBuilder(256);
                GetWindowText(childHwnd, textSb, 256);

                RECT cRect;
                GetWindowRect(childHwnd, out cRect);

                RECT cClientRect;
                GetClientRect(childHwnd, out cClientRect);

                ChildInfo cInfo = new ChildInfo {
                    hwnd = childHwnd.ToInt64(),
                    className = classSb.ToString(),
                    title = textSb.ToString(),
                    left = cRect.Left,
                    top = cRect.Top,
                    right = cRect.Right,
                    bottom = cRect.Bottom,
                    width = cRect.Right - cRect.Left,
                    height = cRect.Bottom - cRect.Top,
                    clientWidth = cClientRect.Right - cClientRect.Left,
                    clientHeight = cClientRect.Bottom - cClientRect.Top,
                    styleHex = string.Format("0x{0:X8}", GetWindowLong(childHwnd, GWL_STYLE)),
                    exStyleHex = string.Format("0x{0:X8}", GetWindowLong(childHwnd, GWL_EXSTYLE))
                };
                res.children.Add(cInfo);
                return true;
            }, IntPtr.Zero);

            return res;
        }
    }
}
"@

if (-not ([System.Management.Automation.PSTypeName]'SilkDiag.Win32HierarchyDiag').Type) {
    Add-Type -TypeDefinition $code
}

$diag = [SilkDiag.Win32HierarchyDiag]::Inspect($Hwnd)
$diag | ConvertTo-Json -Depth 6
