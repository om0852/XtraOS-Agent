// Spike S3 — capture our own window on every monitor and verify capture→screen mapping.
// Two capture paths: PrintWindow(PW_RENDERFULLCONTENT) and screen BitBlt (CopyFromScreen).
// Docs: https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-printwindow
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Text;
using System.Threading;
using System.Windows.Forms;

public static class S3
{
    public static string Run(string outDir)
    {
        string result = null;
        var t = new Thread(() => { try { result = Inner(outDir); } catch (Exception e) { result = "ERROR " + e; } });
        t.SetApartmentState(ApartmentState.STA);
        t.Start(); t.Join();
        return result;
    }

    static string Inner(string outDir)
    {
        Directory.CreateDirectory(outDir);
        Native.SetThreadDpiAwarenessContext(Native.PER_MONITOR_AWARE_V2);
        var sb = new StringBuilder();
        int m = 0;
        foreach (var scr in Screen.AllScreens)
        {
            m++;
            var loc = new Point(scr.WorkingArea.X + 60, scr.WorkingArea.Y + 60);
            using (var w = new SpikeWindow(loc))
            {
                w.Show();
                w.Pump(400);
                uint dpi = Native.GetDpiForWindow(w.Handle);
                Native.RECT wr; Native.GetWindowRect(w.Handle, out wr);
                Point markerScreen = Native.Screen(w.Marker, 0, 0);

                // Path A: PrintWindow into a bitmap the size of the window rect.
                int ww = wr.Right - wr.Left, wh = wr.Bottom - wr.Top;
                string a;
                using (var bmp = new Bitmap(ww, wh, PixelFormat.Format32bppArgb))
                {
                    using (var g = Graphics.FromImage(bmp))
                    {
                        IntPtr hdc = g.GetHdc();
                        Native.PrintWindow(w.Handle, hdc, 2 /* PW_RENDERFULLCONTENT */);
                        g.ReleaseHdc(hdc);
                    }
                    Point f = FindMarker(bmp);
                    if (m == 1) bmp.Save(Path.Combine(outDir, "s3-printwindow-m1.png"), ImageFormat.Png);
                    a = f.X < 0 ? "marker NOT found" :
                        string.Format("delta=({0},{1})", f.X + wr.Left - markerScreen.X, f.Y + wr.Top - markerScreen.Y);
                }

                // Path B: copy the same screen rectangle from the desktop.
                string b;
                using (var bmp = new Bitmap(ww, wh, PixelFormat.Format32bppArgb))
                {
                    using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(wr.Left, wr.Top, 0, 0, new Size(ww, wh));
                    Point f = FindMarker(bmp);
                    b = f.X < 0 ? "marker NOT found" :
                        string.Format("delta=({0},{1})", f.X + wr.Left - markerScreen.X, f.Y + wr.Top - markerScreen.Y);
                }

                sb.AppendLine(string.Format(
                    "monitor {0} bounds={1},{2} {3}x{4} dpi={5} window={6}x{7}  PrintWindow {8}  ScreenCopy {9}",
                    m, scr.Bounds.X, scr.Bounds.Y, scr.Bounds.Width, scr.Bounds.Height, dpi, ww, wh, a, b));
                w.Close();
            }
        }
        return sb.ToString();
    }

    // First magenta (255,0,255) pixel, scanning rows top to bottom.
    static Point FindMarker(Bitmap bmp)
    {
        for (int y = 0; y < bmp.Height; y++)
            for (int x = 0; x < bmp.Width; x++)
            {
                Color c = bmp.GetPixel(x, y);
                if (c.R > 240 && c.G < 15 && c.B > 240) return new Point(x, y);
            }
        return new Point(-1, -1);
    }
}
