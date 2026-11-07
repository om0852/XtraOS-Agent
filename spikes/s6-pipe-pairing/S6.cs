// Spike S6 — named pipe restricted to the current user (+ NETWORK denied) with token pairing.
// Server = native (stands in for xtraos-core); client = Node (stands in for xtraos-mcp).
// Docs: https://learn.microsoft.com/dotnet/api/system.io.pipes.pipesecurity
using System;
using System.IO;
using System.IO.Pipes;
using System.Security.AccessControl;
using System.Security.Cryptography;
using System.Security.Principal;
using System.Text;
using System.Threading;

public static class S6
{
    public static string PipeName;
    public static string Token;
    public static string Sddl;
    static Thread server;
    static volatile bool stop;
    public static int Accepted, Rejected;

    public static void Start()
    {
        var bytes = new byte[32];
        using (var rng = RandomNumberGenerator.Create()) rng.GetBytes(bytes);
        Token = BitConverter.ToString(bytes).Replace("-", "").ToLowerInvariant();
        PipeName = "xtraos-spike-" + Guid.NewGuid().ToString("N").Substring(0, 8);

        var ps = new PipeSecurity();
        ps.AddAccessRule(new PipeAccessRule(WindowsIdentity.GetCurrent().User,
            PipeAccessRights.ReadWrite | PipeAccessRights.CreateNewInstance, AccessControlType.Allow));
        ps.AddAccessRule(new PipeAccessRule(new SecurityIdentifier(WellKnownSidType.NetworkSid, null),
            PipeAccessRights.FullControl, AccessControlType.Deny));
        Sddl = ps.GetSecurityDescriptorSddlForm(AccessControlSections.Access);

        server = new Thread(() => Serve(ps)) { IsBackground = true };
        server.Start();
    }

    public static void Stop() { stop = true; }

    static void Serve(PipeSecurity ps)
    {
        while (!stop)
        {
            using (var pipe = new NamedPipeServerStream(PipeName, PipeDirection.InOut, 1,
                       PipeTransmissionMode.Byte, PipeOptions.None, 4096, 4096, ps))
            {
                try { pipe.WaitForConnection(); } catch { return; }
                if (stop) return;
                var reader = new StreamReader(pipe, new UTF8Encoding(false));
                var writer = new StreamWriter(pipe, new UTF8Encoding(false)) { AutoFlush = true, NewLine = "\n" };
                try
                {
                    bool paired = false;
                    string line;
                    while ((line = reader.ReadLine()) != null)
                    {
                        // Minimal parse is enough for a spike: first message must carry the token.
                        if (!paired)
                        {
                            string tok = Extract(line, "token");
                            if (!SlowEquals(tok, Token))
                            {
                                Rejected++;
                                writer.WriteLine("{\"jsonrpc\":\"2.0\",\"id\":" + (Extract(line, "id") ?? "null") + ",\"error\":{\"code\":\"NOT_PAIRED\"}}");
                                break;
                            }
                            paired = true; Accepted++;
                        }
                        writer.WriteLine("{\"jsonrpc\":\"2.0\",\"id\":" + (Extract(line, "id") ?? "null") + ",\"result\":\"pong\"}");
                    }
                }
                catch (IOException) { }
            }
        }
    }

    static string Extract(string json, string key)
    {
        string k = "\"" + key + "\":";
        int i = json.IndexOf(k, StringComparison.Ordinal);
        if (i < 0) return null;
        i += k.Length;
        if (json[i] == '"') { int j = json.IndexOf('"', i + 1); return json.Substring(i + 1, j - i - 1); }
        int e = i; while (e < json.Length && (char.IsDigit(json[e]) || json[e] == '-')) e++;
        return json.Substring(i, e - i);
    }

    static bool SlowEquals(string a, string b)
    {
        if (a == null || b == null || a.Length != b.Length) return false;
        int diff = 0;
        for (int i = 0; i < a.Length; i++) diff |= a[i] ^ b[i];
        return diff == 0;
    }
}
