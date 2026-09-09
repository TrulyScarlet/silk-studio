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
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace SilkDiag {
    public class Win32PixelDiagV2 {
        [DllImport("user32.dll")]
        public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

        [DllImport("user32.dll")]
        public static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);

        [DllImport("user32.dll")]
        public static extern bool ClientToScreen(IntPtr hWnd, ref POINT lpPoint);

        [DllImport("user32.dll")]
        public static extern bool IsZoomed(IntPtr hWnd);

        [DllImport("user32.dll")]
        public static extern int GetSystemMetrics(int nIndex);

        [DllImport("user32.dll")]
        public static extern IntPtr GetDC(IntPtr hWnd);

        [DllImport("user32.dll")]
        public static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);

        [DllImport("gdi32.dll")]
        public static extern uint GetPixel(IntPtr hDC, int XPos, int YPos);

        [DllImport("gdi32.dll")]
        public static extern IntPtr CreateCompatibleDC(IntPtr hdc);

        [DllImport("gdi32.dll")]
        public static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int nWidth, int nHeight);

        [DllImport("gdi32.dll")]
        public static extern IntPtr SelectObject(IntPtr hdc, IntPtr hgdiobj);

        [DllImport("gdi32.dll")]
        public static extern bool BitBlt(IntPtr hdcDest, int nXDest, int nYDest, int nWidth, int nHeight, IntPtr hdcSrc, int nXSrc, int nYSrc, int dwRop);

        [DllImport("gdi32.dll")]
        public static extern bool DeleteDC(IntPtr hdc);

        [DllImport("gdi32.dll")]
        public static extern bool DeleteObject(IntPtr hObject);

        [DllImport("dwmapi.dll")]
        public static extern int DwmGetWindowAttribute(IntPtr hwnd, int dwAttribute, out RECT pvAttribute, int cbAttribute);

        public const int SRCCOPY = 0x00CC0020;

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

        public class PixelSample {
            public int x;
            public int screenX;
            public int screenY;
            public int r;
            public int g;
            public int b;
            public string hex;
        }

        public class RowSample {
            public int yOffsetFromClientBottom;
            public int screenY;
            public PixelSample[] rowSamples;
            public int whiteCount;
            public int nearWhiteCount;
            public string dominantColor;
        }

        public class DiagResult {
            public int winLeft;
            public int winTop;
            public int winRight;
            public int winBottom;
            public int winWidth;
            public int winHeight;

            public int clientScreenLeft;
            public int clientScreenTop;
            public int clientScreenWidth;
            public int clientScreenHeight;
            public int clientScreenBottom;

            public int dwmLeft;
            public int dwmTop;
            public int dwmRight;
            public int dwmBottom;

            public bool isMaximized;
            public int screenWidth;
            public int screenHeight;
            public string bmpPath;

            public int totalWidthScanned;
            public int whitePixelCountOnClientBottomRow;
            public int nearWhitePixelCountOnClientBottomRow;
            public Dictionary<string, int> clientBottomRowPixelDistribution;
            public RowSample[] samples;
        }

        public static DiagResult Inspect(long hwndLong, string outputPrefix) {
            IntPtr hWnd = new IntPtr(hwndLong);

            RECT rect;
            GetWindowRect(hWnd, out rect);

            RECT clientRect;
            GetClientRect(hWnd, out clientRect);

            POINT clientOrigin = new POINT { X = 0, Y = 0 };
            ClientToScreen(hWnd, ref clientOrigin);

            RECT dwmRect;
            DwmGetWindowAttribute(hWnd, 9, out dwmRect, 16); // DWMWA_EXTENDED_FRAME_BOUNDS

            bool isMax = IsZoomed(hWnd);
            int screenWidth = GetSystemMetrics(0);  // SM_CXSCREEN
            int screenHeight = GetSystemMetrics(1); // SM_CYSCREEN

            int winWidth = rect.Right - rect.Left;
            int winHeight = rect.Bottom - rect.Top;

            int clientWidth = clientRect.Right - clientRect.Left;
            int clientHeight = clientRect.Bottom - clientRect.Top;
            int clientScreenBottom = clientOrigin.Y + clientHeight;

            DiagResult res = new DiagResult();
            res.winLeft = rect.Left;
            res.winTop = rect.Top;
            res.winRight = rect.Right;
            res.winBottom = rect.Bottom;
            res.winWidth = winWidth;
            res.winHeight = winHeight;

            res.clientScreenLeft = clientOrigin.X;
            res.clientScreenTop = clientOrigin.Y;
            res.clientScreenWidth = clientWidth;
            res.clientScreenHeight = clientHeight;
            res.clientScreenBottom = clientScreenBottom;

            res.dwmLeft = dwmRect.Left;
            res.dwmTop = dwmRect.Top;
            res.dwmRight = dwmRect.Right;
            res.dwmBottom = dwmRect.Bottom;
            res.isMaximized = isMax;
            res.screenWidth = screenWidth;
            res.screenHeight = screenHeight;
            res.bmpPath = outputPrefix + "_client_bottom_slice.bmp";
            res.totalWidthScanned = clientWidth;
            res.clientBottomRowPixelDistribution = new Dictionary<string, int>();

            // Capture slice from clientScreenBottom - 30 to clientScreenBottom + 20 (50 pixels high)
            int sliceHeight = 50;
            int captureY = Math.Max(0, clientScreenBottom - 30);
            int captureX = Math.Max(0, clientOrigin.X);
            int captureW = Math.Min(screenWidth, Math.Max(100, clientWidth));

            IntPtr screenDC = GetDC(IntPtr.Zero);
            IntPtr memDC = CreateCompatibleDC(screenDC);
            IntPtr hBitmap = CreateCompatibleBitmap(screenDC, captureW, sliceHeight);
            IntPtr hOld = SelectObject(memDC, hBitmap);

            BitBlt(memDC, 0, 0, captureW, sliceHeight, screenDC, captureX, captureY, SRCCOPY);

            // Read the exact client bottom row: screenY = clientScreenBottom - 1
            int clientBottomRowYInBmp = (clientScreenBottom - 1) - captureY; // which is 29
            if (clientBottomRowYInBmp >= 0 && clientBottomRowYInBmp < sliceHeight) {
                for (int x = 0; x < captureW; x++) {
                    uint colorRef = GetPixel(memDC, x, clientBottomRowYInBmp);
                    int r = (int)(colorRef & 0xFF);
                    int g = (int)((colorRef >> 8) & 0xFF);
                    int b = (int)((colorRef >> 16) & 0xFF);

                    string hex = string.Format("#{0:X2}{1:X2}{2:X2}", r, g, b);
                    if (res.clientBottomRowPixelDistribution.ContainsKey(hex)) {
                        res.clientBottomRowPixelDistribution[hex]++;
                    } else {
                        res.clientBottomRowPixelDistribution[hex] = 1;
                    }

                    if (r == 255 && g == 255 && b == 255) {
                        res.whitePixelCountOnClientBottomRow++;
                    }
                    if (r > 240 && g > 240 && b > 240) {
                        res.nearWhitePixelCountOnClientBottomRow++;
                    }
                }
            }

            // Detailed row sampling around client bottom: offsets -10, -5, -4, -3, -2, -1 (last UI row), 0 (first taskbar row), +1, +2, +5
            int[] sampleYs = new int[] { -10, -5, -4, -3, -2, -1, 0, 1, 2, 5 };
            int[] sampleXs = new int[] { 50, 150, 300, (int)(captureW * 0.25), (int)(captureW * 0.5), (int)(captureW * 0.75), captureW - 50 };

            List<RowSample> rowList = new List<RowSample>();
            foreach (int yOff in sampleYs) {
                int screenY = (clientScreenBottom + yOff);
                if (yOff < 0) {
                    // For negative offsets, -1 is the last pixel row of the client (clientScreenBottom - 1)
                    screenY = clientScreenBottom + yOff; // yOff = -1 -> clientScreenBottom - 1
                } else {
                    // yOff = 0 is clientScreenBottom (first row outside client / taskbar)
                    screenY = clientScreenBottom + yOff;
                }

                int bmpY = screenY - captureY;
                if (bmpY >= 0 && bmpY < sliceHeight) {
                    RowSample row = new RowSample();
                    row.yOffsetFromClientBottom = yOff;
                    row.screenY = screenY;

                    // Scan entire row for white counts
                    Dictionary<string, int> rowDist = new Dictionary<string, int>();
                    for (int x = 0; x < captureW; x++) {
                        uint colorRef = GetPixel(memDC, x, bmpY);
                        int r = (int)(colorRef & 0xFF);
                        int g = (int)((colorRef >> 8) & 0xFF);
                        int b = (int)((colorRef >> 16) & 0xFF);
                        if (r == 255 && g == 255 && b == 255) row.whiteCount++;
                        if (r > 240 && g > 240 && b > 240) row.nearWhiteCount++;

                        string hex = string.Format("#{0:X2}{1:X2}{2:X2}", r, g, b);
                        if (rowDist.ContainsKey(hex)) rowDist[hex]++; else rowDist[hex] = 1;
                    }

                    // find dominant color
                    string dom = "";
                    int maxC = 0;
                    foreach (var kvp in rowDist) {
                        if (kvp.Value > maxC) {
                            maxC = kvp.Value;
                            dom = kvp.Key;
                        }
                    }
                    row.dominantColor = string.Format("{0} ({1}px / {2:F1}%)", dom, maxC, (maxC * 100.0 / captureW));

                    List<PixelSample> pList = new List<PixelSample>();
                    foreach (int x in sampleXs) {
                        if (x >= 0 && x < captureW) {
                            uint colorRef = GetPixel(memDC, x, bmpY);
                            int r = (int)(colorRef & 0xFF);
                            int g = (int)((colorRef >> 8) & 0xFF);
                            int b = (int)((colorRef >> 16) & 0xFF);

                            PixelSample ps = new PixelSample();
                            ps.x = x;
                            ps.screenX = captureX + x;
                            ps.screenY = screenY;
                            ps.r = r;
                            ps.g = g;
                            ps.b = b;
                            ps.hex = string.Format("#{0:X2}{1:X2}{2:X2}", r, g, b);
                            pList.Add(ps);
                        }
                    }
                    row.rowSamples = pList.ToArray();
                    rowList.Add(row);
                }
            }
            res.samples = rowList.ToArray();

            SaveBmp(res.bmpPath, memDC, captureW, sliceHeight);

            SelectObject(memDC, hOld);
            DeleteObject(hBitmap);
            DeleteDC(memDC);
            ReleaseDC(IntPtr.Zero, screenDC);

            return res;
        }

        private static void SaveBmp(string filePath, IntPtr hdc, int width, int height) {
            int rowSize = (width * 3 + 3) & ~3;
            int imageSize = rowSize * height;
            byte[] fileHeader = new byte[14];
            byte[] infoHeader = new byte[40];

            int fileSize = 54 + imageSize;

            fileHeader[0] = (byte)'B';
            fileHeader[1] = (byte)'M';
            BitConverter.GetBytes(fileSize).CopyTo(fileHeader, 2);
            BitConverter.GetBytes(54).CopyTo(fileHeader, 10);

            BitConverter.GetBytes(40).CopyTo(infoHeader, 0);
            BitConverter.GetBytes(width).CopyTo(infoHeader, 4);
            BitConverter.GetBytes(height).CopyTo(infoHeader, 8); // bottom-up
            BitConverter.GetBytes((short)1).CopyTo(infoHeader, 12);
            BitConverter.GetBytes((short)24).CopyTo(infoHeader, 14);
            BitConverter.GetBytes(0).CopyTo(infoHeader, 16);
            BitConverter.GetBytes(imageSize).CopyTo(infoHeader, 20);

            byte[] pixelData = new byte[imageSize];
            for (int y = 0; y < height; y++) {
                int srcY = height - 1 - y;
                int rowOffset = y * rowSize;
                for (int x = 0; x < width; x++) {
                    uint colorRef = GetPixel(hdc, x, srcY);
                    byte r = (byte)(colorRef & 0xFF);
                    byte g = (byte)((colorRef >> 8) & 0xFF);
                    byte b = (byte)((colorRef >> 16) & 0xFF);

                    int pOffset = rowOffset + x * 3;
                    pixelData[pOffset] = b;
                    pixelData[pOffset + 1] = g;
                    pixelData[pOffset + 2] = r;
                }
            }

            using (FileStream fs = new FileStream(filePath, FileMode.Create)) {
                fs.Write(fileHeader, 0, 14);
                fs.Write(infoHeader, 0, 40);
                fs.Write(pixelData, 0, imageSize);
            }
        }
    }
}
"@

if (-not ([System.Management.Automation.PSTypeName]'SilkDiag.Win32PixelDiagV2').Type) {
    Add-Type -TypeDefinition $code
}

$diag = [SilkDiag.Win32PixelDiagV2]::Inspect($Hwnd, $OutputPrefix)
$diag | ConvertTo-Json -Depth 6
