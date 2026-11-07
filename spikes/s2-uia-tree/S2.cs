// Spike S2 — read-only UI Automation walk of real app windows.
// Measures: elements visited, interactive/named elements kept, walk time, compact-output size.
// Docs: https://learn.microsoft.com/dotnet/framework/ui-automation/navigate-among-ui-automation-elements-with-treewalker
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Windows.Automation;

public static class S2
{
    static readonly HashSet<string> Interactive = new HashSet<string>(new[] {
        "Button","Edit","CheckBox","RadioButton","ComboBox","ListItem","MenuItem","TabItem",
        "Hyperlink","TreeItem","Slider","Spinner","SplitButton","Document","DataItem","ScrollBar"
    });

    public static string Run(string[] procNames, int maxDepth, int maxKeep, int msBudget, string outDir)
    {
        var report = new StringBuilder();
        Directory.CreateDirectory(outDir);
        var tops = AutomationElement.RootElement.FindAll(TreeScope.Children, Condition.TrueCondition);
        foreach (AutomationElement w in tops)
        {
            string pname = "";
            try { pname = Process.GetProcessById(w.Current.ProcessId).ProcessName; } catch { continue; }
            bool want = false;
            foreach (var p in procNames) if (string.Equals(p, pname, StringComparison.OrdinalIgnoreCase)) want = true;
            if (!want) continue;

            var sw = Stopwatch.StartNew();
            int visited = 0, kept = 0; bool truncated = false;
            var compact = new StringBuilder();
            Walk(w, 0, maxDepth, maxKeep, msBudget, sw, ref visited, ref kept, ref truncated, compact);
            sw.Stop();

            int chars = compact.Length;
            report.AppendLine(string.Format(
                "{0,-16} visited={1,5} kept={2,4} ms={3,6} chars={4,6} ~tokens={5,5} truncated={6}",
                pname, visited, kept, sw.ElapsedMilliseconds, chars, chars / 4, truncated));
            // Detailed tree stays local (privacy rule): spikes/out/s2-<proc>-<n>.txt
            string file = Path.Combine(outDir, "s2-" + pname + "-" + Guid.NewGuid().ToString("N").Substring(0, 6) + ".txt");
            File.WriteAllText(file, compact.ToString());
        }
        if (report.Length == 0) report.AppendLine("no matching windows open");
        return report.ToString();
    }

    static void Walk(AutomationElement el, int depth, int maxDepth, int maxKeep, int msBudget, Stopwatch sw,
                     ref int visited, ref int kept, ref bool truncated, StringBuilder compact)
    {
        if (depth > maxDepth) return;
        if (sw.ElapsedMilliseconds > msBudget || kept >= maxKeep) { truncated = true; return; }
        visited++;
        try
        {
            var c = el.Current;
            if (!c.IsOffscreen)
            {
                string role = c.ControlType.ProgrammaticName.Replace("ControlType.", "");
                string name = c.Name ?? "";
                if (Interactive.Contains(role) || (role == "Text" && name.Length > 0))
                {
                    kept++;
                    if (name.Length > 60) name = name.Substring(0, 60) + "...";
                    compact.Append('e').Append(kept).Append(' ').Append(role);
                    if (name.Length > 0) compact.Append(" \"").Append(name.Replace("\"", "'")).Append('"');
                    compact.Append('\n');
                }
            }
        }
        catch (ElementNotAvailableException) { return; }

        var walker = TreeWalker.ControlViewWalker;
        AutomationElement child = null;
        try { child = walker.GetFirstChild(el); } catch { return; }
        while (child != null)
        {
            Walk(child, depth + 1, maxDepth, maxKeep, msBudget, sw, ref visited, ref kept, ref truncated, compact);
            if (truncated) return;
            try { child = walker.GetNextSibling(child); } catch { return; }
        }
    }
}
