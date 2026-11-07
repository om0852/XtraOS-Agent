// Shared spike test window: top-most, never activates (WS_EX_NOACTIVATE), so it never steals
// focus or keyboard input from whatever the user is doing.
// Docs: https://learn.microsoft.com/windows/win32/winmsg/extended-window-styles
using System;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Windows.Forms;

public class SpikeWindow : Form
{
    public Panel Marker;
    public Button[] Buttons;
    public int[] Clicks;

    public SpikeWindow(Point location)
    {
        AutoScaleMode = AutoScaleMode.None;
        StartPosition = FormStartPosition.Manual;
        Location = location;
        Size = new Size(420, 260);
        Text = "XtraOS spike window";
        BackColor = Color.White;
        ShowInTaskbar = false;

        Marker = new Panel { Location = new Point(137, 91), Size = new Size(12, 12), BackColor = Color.FromArgb(255, 0, 255) };
        Controls.Add(Marker);

        Buttons = new Button[3];
        Clicks = new int[3];
        Point[] spots = { new Point(20, 20), new Point(260, 30), new Point(150, 170) };
        for (int i = 0; i < 3; i++)
        {
            int idx = i;
            var b = new Button { Text = "B" + i, Location = spots[i], Size = new Size(110, 40), TabStop = false };
            b.Click += delegate { Clicks[idx]++; };
            Buttons[i] = b;
            Controls.Add(b);
        }
    }

    protected override bool ShowWithoutActivation { get { return true; } }

    protected override CreateParams CreateParams
    {
        get
        {
            var cp = base.CreateParams;
            cp.ExStyle |= 0x08000000; // WS_EX_NOACTIVATE
            cp.ExStyle |= 0x00000008; // WS_EX_TOPMOST
            return cp;
        }
    }

    public void Pump(int ms)
    {
        var until = Environment.TickCount + ms;
        while (Environment.TickCount < until) { Application.DoEvents(); System.Threading.Thread.Sleep(10); }
    }
}

public static class Native
{
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    // Per-monitor DPI awareness v2 for this thread: coordinates are physical pixels.
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setthreaddpiawarenesscontext
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
    public static readonly IntPtr PER_MONITOR_AWARE_V2 = new IntPtr(-4);

    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref POINT p);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vk);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int idx);
    [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint n, INPUT[] inputs, int size);

    [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] public struct InputUnion { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public InputUnion u; }

    public const uint INPUT_MOUSE = 0;
    public const uint MOUSEEVENTF_MOVE = 0x0001, MOUSEEVENTF_LEFTDOWN = 0x0002, MOUSEEVENTF_LEFTUP = 0x0004,
                      MOUSEEVENTF_VIRTUALDESK = 0x4000, MOUSEEVENTF_ABSOLUTE = 0x8000;
    public const int SM_XVIRTUALSCREEN = 76, SM_YVIRTUALSCREEN = 77, SM_CXVIRTUALSCREEN = 78, SM_CYVIRTUALSCREEN = 79;
    public const int VK_LBUTTON = 0x01, VK_RBUTTON = 0x02, VK_MBUTTON = 0x04;

    public static Point Screen(Control c, int x, int y)
    {
        var p = new POINT { X = x, Y = y };
        ClientToScreen(c.Handle, ref p);
        return new Point(p.X, p.Y);
    }
}
