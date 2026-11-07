// Read-only diagnosis for S9: compare UIA BoundingRectangle with Win32 physical rects. No input.
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Automation;

public static class Diag
{
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint f);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")] static extern int GetAwarenessFromDpiAwarenessContext(IntPtr c);

    public static string Run(int hwndInt)
    {
        string r = null;
        var t = new Thread(() => { try { r = Inner(hwndInt); } catch (Exception e) { r = "ERROR " + e.Message; } });
        t.SetApartmentState(ApartmentState.STA); t.Start(); t.Join(); return r;
    }

    static string Cls(IntPtr h) { var sb = new StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }

    static string Inner(int hwndInt)
    {
        Native.SetThreadDpiAwarenessContext(Native.PER_MONITOR_AWARE_V2);
        var sb = new StringBuilder();
        sb.AppendLine("thread awareness=" + GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) + " (2=per-monitor)");
        IntPtr hwnd = new IntPtr(hwndInt);
        Native.RECT wr; Native.GetWindowRect(hwnd, out wr);
        sb.AppendLine(string.Format("Win32 window rect (physical): {0},{1} {2}x{3}", wr.Left, wr.Top, wr.Right - wr.Left, wr.Bottom - wr.Top));
        var calc = AutomationElement.FromHandle(hwnd);
        var br = calc.Current.BoundingRectangle;
        sb.AppendLine(string.Format("UIA window rect             : {0},{1} {2}x{3}", br.X, br.Y, br.Width, br.Height));
        var el = calc.FindFirst(TreeScope.Descendants, new PropertyCondition(AutomationElement.AutomationIdProperty, "clearButton"));
        if (el == null) { sb.AppendLine("clearButton not found"); return sb.ToString(); }
        var b = el.Current.BoundingRectangle;
        sb.AppendLine(string.Format("UIA clearButton rect        : {0},{1} {2}x{3}", b.X, b.Y, b.Width, b.Height));
        var p = new Native.POINT { X = (int)(b.X + b.Width / 2), Y = (int)(b.Y + b.Height / 2) };
        IntPtr hit = Native.WindowFromPoint(p);
        IntPtr root = GetAncestor(hit, 2);
        sb.AppendLine(string.Format("WindowFromPoint(centre)     : class={0} root={1} ourFrame={2} rootClass={3}",
            Cls(hit), root.ToInt64(), hwnd.ToInt64(), Cls(root)));
        return sb.ToString();
    }
}
