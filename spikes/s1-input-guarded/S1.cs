// Spike S1 — guarded SendInput clicks on OUR OWN non-activating window, on every monitor.
// Safety: before every event we check (1) the point is over our button, (2) no mouse button is
// held by the user, (3) the cursor is still where we last put it. Any failure aborts. The user's
// cursor position is restored at the end. Our window never takes focus (WS_EX_NOACTIVATE).
// Docs: https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-sendinput
//       https://learn.microsoft.com/windows/win32/api/winuser/ns-winuser-mouseinput (ABSOLUTE|VIRTUALDESK)
using System;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

public static class S1
{
    public static string Run()
    {
        string result = null;
        var t = new Thread(() => { try { result = Inner(); } catch (Exception e) { result = "ERROR " + e; } });
        t.SetApartmentState(ApartmentState.STA);
        t.Start(); t.Join();
        return result;
    }

    static string Inner()
    {
        Native.SetThreadDpiAwarenessContext(Native.PER_MONITOR_AWARE_V2);
        var sb = new StringBuilder();
        Native.POINT original; Native.GetCursorPos(out original);
        Native.POINT lastSet = original;
        bool aborted = false;
        int m = 0;
        try
        {
            foreach (var scr in Screen.AllScreens)
            {
                m++;
                using (var w = new SpikeWindow(new Point(scr.WorkingArea.X + 60, scr.WorkingArea.Y + 60)))
                {
                    w.Show(); w.Pump(300);
                    uint dpi = Native.GetDpiForWindow(w.Handle);
                    for (int i = 0; i < w.Buttons.Length; i++)
                    {
                        var btn = w.Buttons[i];
                        Point target = Native.Screen(btn, btn.Width / 2, btn.Height / 2);

                        string why = Guard(btn.Handle, target, lastSet);
                        if (why != null) { sb.AppendLine("ABORT before move: " + why); aborted = true; break; }

                        var t0 = DateTime.UtcNow;
                        Move(target); w.Pump(30);
                        Native.POINT now; Native.GetCursorPos(out now);
                        lastSet = now;
                        int errX = now.X - target.X, errY = now.Y - target.Y;

                        why = Guard(btn.Handle, target, lastSet);
                        if (why != null) { sb.AppendLine("ABORT before click: " + why); aborted = true; break; }

                        int before = w.Clicks[i];
                        int othersBefore = 0; for (int j = 0; j < w.Clicks.Length; j++) if (j != i) othersBefore += w.Clicks[j];
                        Click(); w.Pump(60);
                        double ms = (DateTime.UtcNow - t0).TotalMilliseconds;
                        bool ok = w.Clicks[i] == before + 1;
                        // stray = clicks that landed on a DIFFERENT button during this click only
                        int othersAfter = 0; for (int j = 0; j < w.Clicks.Length; j++) if (j != i) othersAfter += w.Clicks[j];
                        int stray = othersAfter - othersBefore;

                        sb.AppendLine(string.Format(
                            "monitor {0} dpi={1} B{2} target=({3},{4}) cursorErr=({5},{6}) clicked={7} stray={8} ms={9:0}",
                            m, dpi, i, target.X, target.Y, errX, errY, ok, stray, ms));
                    }
                    w.Close();
                }
                if (aborted) break;
            }
        }
        finally
        {
            Native.SetCursorPos(original.X, original.Y); // give the user their cursor back
        }
        sb.AppendLine(aborted ? "RESULT: aborted by safety guard (nothing else was touched)" : "RESULT: completed");
        return sb.ToString();
    }

    static string Guard(IntPtr expectedHwnd, Point target, Native.POINT lastSet)
    {
        if ((Native.GetAsyncKeyState(Native.VK_LBUTTON) & 0x8000) != 0 ||
            (Native.GetAsyncKeyState(Native.VK_RBUTTON) & 0x8000) != 0 ||
            (Native.GetAsyncKeyState(Native.VK_MBUTTON) & 0x8000) != 0)
            return "a mouse button is held by the user";
        Native.POINT cur; Native.GetCursorPos(out cur);
        if (Math.Abs(cur.X - lastSet.X) > 3 || Math.Abs(cur.Y - lastSet.Y) > 3)
            return "user moved the mouse";
        var p = new Native.POINT { X = target.X, Y = target.Y };
        if (Native.WindowFromPoint(p) != expectedHwnd)
            return "target point is not over our own button";
        return null;
    }

    static void Move(Point target)
    {
        int vx = Native.GetSystemMetrics(Native.SM_XVIRTUALSCREEN), vy = Native.GetSystemMetrics(Native.SM_YVIRTUALSCREEN);
        int vw = Native.GetSystemMetrics(Native.SM_CXVIRTUALSCREEN), vh = Native.GetSystemMetrics(Native.SM_CYVIRTUALSCREEN);
        // Normalise to 0..65535 across the virtual desktop (rounded to the pixel centre).
        int nx = (int)Math.Round((target.X - vx) * 65535.0 / (vw - 1));
        int ny = (int)Math.Round((target.Y - vy) * 65535.0 / (vh - 1));
        Send(new Native.MOUSEINPUT { dx = nx, dy = ny,
            dwFlags = Native.MOUSEEVENTF_MOVE | Native.MOUSEEVENTF_ABSOLUTE | Native.MOUSEEVENTF_VIRTUALDESK });
    }

    static void Click()
    {
        Send(new Native.MOUSEINPUT { dwFlags = Native.MOUSEEVENTF_LEFTDOWN });
        Thread.Sleep(15);
        Send(new Native.MOUSEINPUT { dwFlags = Native.MOUSEEVENTF_LEFTUP });
    }

    static void Send(Native.MOUSEINPUT mi)
    {
        var input = new Native.INPUT[] { new Native.INPUT { type = Native.INPUT_MOUSE, u = new Native.InputUnion { mi = mi } } };
        Native.SendInput(1, input, Marshal.SizeOf(typeof(Native.INPUT)));
    }
}
