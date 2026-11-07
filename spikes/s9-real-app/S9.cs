// Spike S9 — operate a REAL third-party app (Windows Calculator) launched by the spike:
// find controls via UI Automation from outside the process, click them with real input,
// type with real keyboard input, verify the result, close it. Repeats on every monitor.
// Safety: acts only on the Calculator window this spike launched (a pre-existing one is
// never touched); click target must be inside that window; keys only while it is the
// foreground window; aborts on user mouse movement or held buttons; restores cursor and the
// previously active window.
// Docs: https://learn.microsoft.com/windows/win32/winauto/entry-uiauto-win32
//       https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-sendinput
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Automation;
using System.Windows.Forms;

public static class S9
{
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
    const uint GA_ROOT = 2, SWP_NOSIZE = 0x0001, SWP_NOZORDER = 0x0004;
    const int SW_RESTORE = 9;

    // Foreground-lock aware focus without synthesising keystrokes (an Alt tap would toggle the
    // menu bar of whatever app the user is in). Docs:
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setforegroundwindow (restrictions)
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-attachthreadinput
    static bool Focus(IntPtr hwnd)
    {
        if (GetForegroundWindow() == hwnd) return true;
        IntPtr fg = GetForegroundWindow();
        uint fgThread = GetWindowThreadProcessId(fg, IntPtr.Zero);
        uint me = GetCurrentThreadId();
        bool attached = fgThread != 0 && fgThread != me && AttachThreadInput(me, fgThread, true);
        try { BringWindowToTop(hwnd); SetForegroundWindow(hwnd); }
        finally { if (attached) AttachThreadInput(me, fgThread, false); }
        Thread.Sleep(150);
        return GetForegroundWindow() == hwnd;
    }
    const uint INPUT_KEYBOARD = 1, KEYEVENTF_KEYUP = 0x0002;

    static StringBuilder log = new StringBuilder();
    static Native.POINT lastSet;

    public static string Run()
    {
        string result = null;
        var t = new Thread(() => { try { result = Inner(); } catch (Exception e) { result = log + "ERROR " + e.Message; } });
        t.SetApartmentState(ApartmentState.STA);
        t.Start(); t.Join();
        return result;
    }

    static string Inner()
    {
        Native.SetThreadDpiAwarenessContext(Native.PER_MONITOR_AWARE_V2);
        Native.POINT originalCursor; Native.GetCursorPos(out originalCursor);
        lastSet = originalCursor;
        IntPtr originalFg = GetForegroundWindow();

        var before = new HashSet<int>();
        foreach (AutomationElement w in Calculators()) before.Add(w.Current.NativeWindowHandle);

        log.AppendLine("monitors: " + Screen.AllScreens.Length);
        var sw = Stopwatch.StartNew();
        Process.Start("calc.exe");
        AutomationElement calc = null;
        while (sw.ElapsedMilliseconds < 10000 && calc == null)
        {
            Thread.Sleep(200);
            foreach (AutomationElement w in Calculators())
                if (!before.Contains(w.Current.NativeWindowHandle)) { calc = w; break; }
        }
        if (calc == null) return log + "ABORT: no NEW Calculator window appeared (an existing one is never touched)";
        IntPtr hwnd = new IntPtr(calc.Current.NativeWindowHandle);
        log.AppendLine(string.Format("launched Calculator window in {0} ms", sw.ElapsedMilliseconds));

        try
        {
            int m = 0;
            foreach (var scr in Screen.AllScreens)
            {
                m++;
                ShowWindow(hwnd, SW_RESTORE); // Calculator may open maximized
                Thread.Sleep(300);
                SetWindowPos(hwnd, IntPtr.Zero, scr.WorkingArea.X + 80, scr.WorkingArea.Y + 80, 480, 760, SWP_NOZORDER);
                Thread.Sleep(600);
                bool focused = Focus(hwnd);
                log.AppendLine("   brought to front: " + focused);
                if (!focused) return log + "ABORT: could not bring our Calculator to the front (Windows focus lock)";
                uint dpi = Native.GetDpiForWindow(hwnd);
                log.AppendLine(string.Format("-- monitor {0} ({1}x{2} at {3},{4}) window dpi={5}",
                    m, scr.Bounds.Width, scr.Bounds.Height, scr.Bounds.X, scr.Bounds.Y, dpi));

                // A) Clicks found via UI Automation: C, 7, +, 8, =  → expect 15
                string[] ids = { "clearButton", "num7Button", "plusButton", "num8Button", "equalButton" };
                foreach (var id in ids)
                {
                    var el = WaitFor(calc, id, 5000);
                    if (el == null) return log + "ABORT: control not found within 5 s: " + id;
                    var r = el.Current.BoundingRectangle;
                    var target = new Point((int)(r.X + r.Width / 2), (int)(r.Y + r.Height / 2));
                    string why = ClickGuard(hwnd, target);
                    if (why != null) return log + "ABORT before click " + id + ": " + why;
                    var t0 = Stopwatch.StartNew();
                    Move(target); Thread.Sleep(25);
                    Native.POINT now; Native.GetCursorPos(out now); lastSet = now;
                    why = ClickGuard(hwnd, target);
                    if (why != null) return log + "ABORT before click " + id + ": " + why;
                    Click(); Thread.Sleep(120);
                    log.AppendLine(string.Format("   click {0,-12} at ({1},{2}) cursorErr=({3},{4}) {5}ms",
                        id, target.X, target.Y, now.X - target.X, now.Y - target.Y, t0.ElapsedMilliseconds));
                }
                string res = Display(calc);
                log.AppendLine("   clicks result: \"" + res + "\" expected 15 -> " + (res.EndsWith(" 15") ? "PASS" : "FAIL"));

                // B) Real keyboard: Esc, 1, 2, *, 3, Enter  → expect 36
                ushort[] keys = { 0x1B, 0x31, 0x32, 0x6A /*VK_MULTIPLY*/, 0x33, 0x0D };
                foreach (var vk in keys)
                {
                    if (GetForegroundWindow() != hwnd) return log + "ABORT: Calculator lost focus; no keys sent elsewhere";
                    Key(vk); Thread.Sleep(60);
                }
                res = Display(calc);
                log.AppendLine("   keyboard result: \"" + res + "\" expected 36 -> " + (res.EndsWith(" 36") ? "PASS" : "FAIL"));
            }
        }
        finally
        {
            try { ((WindowPattern)calc.GetCurrentPattern(WindowPattern.Pattern)).Close(); log.AppendLine("closed our Calculator"); }
            catch (Exception e) { log.AppendLine("could not close Calculator: " + e.Message); }
            Native.SetCursorPos(originalCursor.X, originalCursor.Y);
            if (originalFg != IntPtr.Zero) SetForegroundWindow(originalFg);
        }
        return log.ToString(); // after cleanup, so the "closed" line is included
    }

    static AutomationElementCollection Calculators()
    {
        return AutomationElement.RootElement.FindAll(TreeScope.Children,
            new PropertyCondition(AutomationElement.NameProperty, "Calculator"));
    }

    // UWP apps show their frame before the content tree is ready: poll instead of sleeping.
    static AutomationElement WaitFor(AutomationElement root, string id, int ms)
    {
        var sw = Stopwatch.StartNew();
        var cond = new PropertyCondition(AutomationElement.AutomationIdProperty, id);
        while (sw.ElapsedMilliseconds < ms)
        {
            AutomationElement el = null;
            try { el = root.FindFirst(TreeScope.Descendants, cond); } catch (ElementNotAvailableException) { }
            if (el != null) return el;
            Thread.Sleep(100);
        }
        return null;
    }

    static string Display(AutomationElement calc)
    {
        var d = calc.FindFirst(TreeScope.Descendants, new PropertyCondition(AutomationElement.AutomationIdProperty, "CalculatorResults"));
        return d == null ? "(display not found)" : d.Current.Name;
    }

    static string ClickGuard(IntPtr hwnd, Point target)
    {
        if ((Native.GetAsyncKeyState(Native.VK_LBUTTON) & 0x8000) != 0 || (Native.GetAsyncKeyState(Native.VK_RBUTTON) & 0x8000) != 0)
            return "a mouse button is held by the user";
        Native.POINT cur; Native.GetCursorPos(out cur);
        if (Math.Abs(cur.X - lastSet.X) > 3 || Math.Abs(cur.Y - lastSet.Y) > 3) return "user moved the mouse";
        var p = new Native.POINT { X = target.X, Y = target.Y };
        if (GetAncestor(Native.WindowFromPoint(p), GA_ROOT) != hwnd) return "target point is not inside our Calculator window";
        return null;
    }

    static void Move(Point target)
    {
        int vx = Native.GetSystemMetrics(Native.SM_XVIRTUALSCREEN), vy = Native.GetSystemMetrics(Native.SM_YVIRTUALSCREEN);
        int vw = Native.GetSystemMetrics(Native.SM_CXVIRTUALSCREEN), vh = Native.GetSystemMetrics(Native.SM_CYVIRTUALSCREEN);
        SendMouse(new Native.MOUSEINPUT {
            dx = (int)Math.Round((target.X - vx) * 65535.0 / (vw - 1)),
            dy = (int)Math.Round((target.Y - vy) * 65535.0 / (vh - 1)),
            dwFlags = Native.MOUSEEVENTF_MOVE | Native.MOUSEEVENTF_ABSOLUTE | Native.MOUSEEVENTF_VIRTUALDESK });
    }

    static void Click()
    {
        SendMouse(new Native.MOUSEINPUT { dwFlags = Native.MOUSEEVENTF_LEFTDOWN });
        Thread.Sleep(15);
        SendMouse(new Native.MOUSEINPUT { dwFlags = Native.MOUSEEVENTF_LEFTUP });
    }

    static void SendMouse(Native.MOUSEINPUT mi)
    {
        var a = new[] { new Native.INPUT { type = Native.INPUT_MOUSE, u = new Native.InputUnion { mi = mi } } };
        Native.SendInput(1, a, Marshal.SizeOf(typeof(Native.INPUT)));
    }

    static void Key(ushort vk)
    {
        var a = new[] {
            new Native.INPUT { type = INPUT_KEYBOARD, u = new Native.InputUnion { ki = new Native.KEYBDINPUT { wVk = vk } } },
            new Native.INPUT { type = INPUT_KEYBOARD, u = new Native.InputUnion { ki = new Native.KEYBDINPUT { wVk = vk, dwFlags = KEYEVENTF_KEYUP } } }
        };
        Native.SendInput(2, a, Marshal.SizeOf(typeof(Native.INPUT)));
    }
}
