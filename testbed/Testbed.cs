// XtraOS Testbed — a window with known controls for interactive input tests (ADR 0010).
// Compiled with the .NET Framework csc.exe that ships with Windows (C# 5). Per-monitor DPI
// aware v2, so every rectangle it reports is in physical screen pixels, like the core.
//
// Control channel (one line per command on stdin, one JSON line per reply on stdout):
// Started with "--control": control channel on stdin/stdout. Without it: a normal app.
//   rects  -> {"hwnd":N,"controls":{"btnA":[x,y,w,h],...}}
//   state  -> {"clicks":{...},"pad":[...],"text":"...","multi":"...","passwordLen":N,
//              "listTop":N,"dropped":"...","path":{...},"keys":[...]}
//   reset  -> {"ok":true}        quit -> {"ok":true} and exits
//   fillpassword -> puts a known secret in the password box (M4 masking test, no typing)
// "--opaque": a single custom-painted surface with no accessible controls (M4 opaque test).
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

static class Program
{
    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    static readonly object OutLock = new object();

    [STAThread]
    static void Main(string[] args)
    {
        SetProcessDpiAwarenessContext(new IntPtr(-4)); // PER_MONITOR_AWARE_V2, before any window
        // Report non-ASCII text faithfully. A winexe has no console, so Console.OutputEncoding
        // would throw; wrap the raw (redirected) stdout stream in a UTF-8 writer instead.
        Console.SetOut(new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false)) { AutoFlush = true });
        Application.EnableVisualStyles();
        int x = args.Length > 0 ? int.Parse(args[0]) : 120;
        int y = args.Length > 1 ? int.Parse(args[1]) : 120;
        Form form = Array.IndexOf(args, "--opaque") >= 0 ? (Form)new OpaqueForm(new Point(x, y)) : new Testbed(new Point(x, y));
        // The stdin control channel is opt-in via "--control" (the test harness passes it).
        // Without it (e.g. started by launch_app via ShellExecute) behave like a normal app and
        // stay open until closed (ADR 0012). Console.IsInputRedirected is NOT usable here: in a
        // winexe with no console the missing stdin handle counts as "redirected", ReadLine
        // returns null at once and the window would close immediately (found by win01).
        bool controlled = Array.IndexOf(args, "--control") >= 0;
        var reader = new Thread(() => ReadLoop(form as Testbed)) { IsBackground = true };
        form.Shown += delegate { if (controlled && form is Testbed) { reader.Start(); Out("{\"ready\":true}"); } };
        Application.Run(form);
    }

    public static void Out(string s)
    {
        lock (OutLock) { Console.Out.WriteLine(s); Console.Out.Flush(); }
    }

    static void ReadLoop(Testbed form)
    {
        string line;
        while ((line = Console.In.ReadLine()) != null)
        {
            // Commands are plain lowercase words; keep only a-z so a UTF-8 BOM (which some
            // writers prepend, and the console decodes as stray characters) cannot break them.
            var sbCmd = new StringBuilder();
            foreach (char ch in line) if (ch >= 'a' && ch <= 'z') sbCmd.Append(ch);
            string cmd = sbCmd.ToString();
            string reply = (string)form.Invoke(new Func<string>(() => form.Command(cmd)));
            Out(reply);
            if (cmd == "quit") break;
        }
        try { form.BeginInvoke(new Action(() => form.Close())); } catch (InvalidOperationException) { }
    }
}

class Testbed : Form
{
    readonly Dictionary<string, Control> named = new Dictionary<string, Control>();
    readonly Dictionary<string, int> clicks = new Dictionary<string, int>();
    readonly List<string> padEvents = new List<string>();
    readonly List<string> keys = new List<string>();
    TextBox text, multi, password;
    ListBox list;
    Panel pad, dropTarget, drawPad;
    Label dragSource;
    string dropped = "";
    bool drawing;
    int pathCount;
    Point pathFirst, pathLast;

    public Testbed(Point location)
    {
        AutoScaleMode = AutoScaleMode.None;
        StartPosition = FormStartPosition.Manual;
        Location = location;
        Size = new Size(760, 520);
        Text = "XtraOS Testbed";
        TopMost = true;
        KeyPreview = true;
        BackColor = Color.White;

        foreach (var spec in new[] { new { N = "btnA", X = 20 }, new { N = "btnB", X = 150 }, new { N = "btnC", X = 280 } })
        {
            string name = spec.N;
            var b = new Button { Text = name, Location = new Point(spec.X, 20), Size = new Size(110, 44), TabStop = false };
            clicks[name] = 0;
            b.Click += delegate { clicks[name]++; };
            Reg(name, b);
        }

        // A "Send" button (M5 policy tests: message_send must be confirmed by the user).
        var send = new Button { Text = "Send", Location = new Point(600, 20), Size = new Size(110, 44), TabStop = false };
        clicks["send"] = 0;
        send.Click += delegate { clicks["send"]++; };
        Reg("send", send);

        pad = new Panel { Location = new Point(420, 20), Size = new Size(150, 80), BackColor = Color.LightSteelBlue };
        pad.MouseDown += (s, e) => padEvents.Add(e.Button + ":" + e.Clicks);
        Reg("pad", pad);

        text = new TextBox { Location = new Point(20, 90), Width = 360 };
        Reg("text", text);
        multi = new TextBox { Multiline = true, AcceptsTab = true, AcceptsReturn = true, Location = new Point(20, 130), Size = new Size(360, 70) };
        Reg("multi", multi);
        password = new TextBox { UseSystemPasswordChar = true, Location = new Point(20, 215), Width = 360 };
        Reg("password", password);

        list = new ListBox { Location = new Point(420, 120), Size = new Size(150, 180), IntegralHeight = false };
        for (int i = 1; i <= 200; i++) list.Items.Add("Item " + i);
        Reg("list", list);

        dragSource = new Label { Text = "DRAG ME", Location = new Point(20, 265), Size = new Size(120, 50), BackColor = Color.Khaki, TextAlign = ContentAlignment.MiddleCenter };
        dragSource.MouseDown += (s, e) => { if (e.Button == MouseButtons.Left) dragSource.DoDragDrop("payload-42", DragDropEffects.Copy); };
        Reg("dragSource", dragSource);

        dropTarget = new Panel { Location = new Point(200, 255), Size = new Size(180, 70), BackColor = Color.Honeydew, AllowDrop = true };
        dropTarget.DragEnter += (s, e) => { e.Effect = e.Data.GetDataPresent(DataFormats.Text) || e.Data.GetDataPresent(DataFormats.FileDrop) ? DragDropEffects.Copy : DragDropEffects.None; };
        dropTarget.DragDrop += (s, e) =>
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                var files = (string[])e.Data.GetData(DataFormats.FileDrop);
                dropped = "file:" + System.IO.Path.GetFileName(files[0]);
            }
            else dropped = (string)e.Data.GetData(DataFormats.Text);
        };
        Reg("dropTarget", dropTarget);

        drawPad = new Panel { Location = new Point(20, 345), Size = new Size(360, 110), BackColor = Color.MistyRose };
        drawPad.MouseDown += (s, e) => { drawing = true; pathCount = 1; pathFirst = drawPad.PointToScreen(e.Location); pathLast = pathFirst; };
        drawPad.MouseMove += (s, e) => { if (drawing) { pathCount++; pathLast = drawPad.PointToScreen(e.Location); } };
        drawPad.MouseUp += (s, e) => { drawing = false; pathLast = drawPad.PointToScreen(e.Location); };
        Reg("drawPad", drawPad);

        KeyDown += (s, e) =>
        {
            if (e.KeyCode == Keys.ControlKey || e.KeyCode == Keys.ShiftKey || e.KeyCode == Keys.Menu || e.KeyCode == Keys.LWin) return;
            var sb = new StringBuilder();
            if (e.Control) sb.Append("Ctrl+");
            if (e.Shift) sb.Append("Shift+");
            if (e.Alt) sb.Append("Alt+");
            sb.Append(e.KeyCode);
            keys.Add(sb.ToString());
        };
    }

    // AccessibleName = the control's test name, so UI Automation shows e.g. Edit "text" (M4).
    void Reg(string name, Control c) { c.AccessibleName = name; named[name] = c; Controls.Add(c); }

    public string Command(string cmd)
    {
        switch (cmd)
        {
            case "rects": return Rects();
            case "state": return State();
            case "reset": Reset(); return "{\"ok\":true}";
            case "fillpassword": password.Text = "hunter2-secret"; return "{\"ok\":true}";
            case "quit": return "{\"ok\":true}";
            default: return "{\"error\":\"unknown command\"}";
        }
    }

    string Rects()
    {
        var sb = new StringBuilder("{\"hwnd\":").Append(Handle.ToInt64()).Append(",\"controls\":{");
        bool first = true;
        foreach (var kv in named)
        {
            Rectangle r = kv.Value.RectangleToScreen(kv.Value.ClientRectangle);
            if (!first) sb.Append(',');
            first = false;
            sb.Append('"').Append(kv.Key).Append("\":[").Append(r.X).Append(',').Append(r.Y).Append(',').Append(r.Width).Append(',').Append(r.Height).Append(']');
        }
        return sb.Append("}}").ToString();
    }

    string State()
    {
        var sb = new StringBuilder("{\"clicks\":{");
        bool first = true;
        foreach (var kv in clicks)
        {
            if (!first) sb.Append(',');
            first = false;
            sb.Append('"').Append(kv.Key).Append("\":").Append(kv.Value);
        }
        sb.Append("},\"pad\":").Append(Arr(padEvents));
        sb.Append(",\"text\":").Append(Str(text.Text));
        sb.Append(",\"multi\":").Append(Str(multi.Text));
        sb.Append(",\"passwordLen\":").Append(password.Text.Length);
        sb.Append(",\"listTop\":").Append(list.TopIndex);
        sb.Append(",\"dropped\":").Append(Str(dropped));
        sb.Append(",\"path\":{\"count\":").Append(pathCount)
          .Append(",\"first\":[").Append(pathFirst.X).Append(',').Append(pathFirst.Y)
          .Append("],\"last\":[").Append(pathLast.X).Append(',').Append(pathLast.Y).Append("]}");
        sb.Append(",\"keys\":").Append(Arr(keys));
        return sb.Append('}').ToString();
    }

    void Reset()
    {
        foreach (var k in new List<string>(clicks.Keys)) clicks[k] = 0;
        padEvents.Clear(); keys.Clear();
        text.Text = ""; multi.Text = ""; password.Text = "";
        list.TopIndex = 0; dropped = ""; pathCount = 0; pathFirst = Point.Empty; pathLast = Point.Empty;
    }

    static string Arr(List<string> items)
    {
        var sb = new StringBuilder("[");
        for (int i = 0; i < items.Count; i++) { if (i > 0) sb.Append(','); sb.Append(Str(items[i])); }
        return sb.Append(']').ToString();
    }

    static string Str(string s)
    {
        var sb = new StringBuilder("\"");
        foreach (char c in s)
        {
            switch (c)
            {
                case '"': sb.Append("\\\""); break;
                case '\\': sb.Append("\\\\"); break;
                case '\n': sb.Append("\\n"); break;
                case '\r': sb.Append("\\r"); break;
                case '\t': sb.Append("\\t"); break;
                default:
                    if (c < 0x20) sb.Append("\\u").Append(((int)c).ToString("x4"));
                    else sb.Append(c);
                    break;
            }
        }
        return sb.Append('"').ToString();
    }
}

// A window whose content is drawn by hand: UI Automation sees no controls inside (like canvas
// apps, games and some Electron apps).
class OpaqueForm : Form
{
    public OpaqueForm(Point location)
    {
        AutoScaleMode = AutoScaleMode.None;
        StartPosition = FormStartPosition.Manual;
        Location = location;
        Size = new Size(640, 420);
        Text = "XtraOS Opaque";
        BackColor = Color.White;
        Paint += (s, e) =>
        {
            e.Graphics.FillRectangle(Brushes.MediumSeaGreen, 40, 40, 200, 80);
            e.Graphics.FillEllipse(Brushes.Coral, 300, 60, 120, 120);
            e.Graphics.DrawString("canvas only", Font, Brushes.Black, 40, 140);
        };
    }
}
