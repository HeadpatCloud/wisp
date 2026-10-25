# Runs at every logon of wisp (the WispProbe scheduled task, see New-TestVm.ps1): a full-screen
# window with a focused text box that writes what arrives into files in C:\wisp, which the host
# reads with PowerShell Direct. README.md describes the files.

Add-Type -ReferencedAssemblies System.Windows.Forms, System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

public class Probe : Form, IMessageFilter {
    const string Dir = @"C:\wisp\";
    static readonly Encoding Utf8 = new UTF8Encoding(false);

    [DllImport("user32.dll")] static extern bool AddClipboardFormatListener(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] static extern IntPtr GetKeyboardLayout(uint thread);

    readonly RichTextBox box = new RichTextBox();
    readonly System.Windows.Forms.Timer poll = new System.Windows.Forms.Timer();
    DateTime clipboardFile;
    DateTime colourFile;
    Size screen;

    public static void Main() {
        SetProcessDPIAware();
        Application.EnableVisualStyles();
        var probe = new Probe();
        Application.AddMessageFilter(probe);
        Application.Run(probe);
    }

    Probe() {
        Text = "wisp probe";
        FormBorderStyle = FormBorderStyle.None;
        TopMost = true;
        BackColor = Color.FromArgb(0x30, 0x60, 0x90);
        StartPosition = FormStartPosition.Manual;

        box.Bounds = new Rectangle(20, 20, 600, 120);
        box.BackColor = Color.White;
        box.ForeColor = Color.Black;
        box.BorderStyle = BorderStyle.None;
        box.Font = new Font("Consolas", 16);
        box.TextChanged += delegate { Write("typed.txt", box.Text); };
        Controls.Add(box);

        Color[] bars = { Color.Red, Color.Lime, Color.Blue, Color.Yellow };
        for (int i = 0; i < bars.Length; i++) {
            var bar = new Panel();
            bar.Bounds = new Rectangle(20 + i * 160, 160, 160, 80);
            bar.BackColor = bars[i];
            Controls.Add(bar);
        }

        clipboardFile = Stamp("set-clipboard.txt");
        colourFile = Stamp("colour.txt");
        poll.Interval = 200;
        poll.Tick += delegate { Poll(); };
        poll.Start();
        Activated += delegate { box.Focus(); };
    }

    protected override void OnHandleCreated(EventArgs e) {
        base.OnHandleCreated(e);
        AddClipboardFormatListener(Handle);
        Write("typed.txt", "");
        // Windows takes the client's layout at logon and keeps it for the session.
        Write("layout.txt", (GetKeyboardLayout(0).ToInt64() & 0xFFFF).ToString("x4"));
        FollowScreen();
    }

    protected override void WndProc(ref Message m) {
        const int WM_CLIPBOARDUPDATE = 0x31D;
        if (m.Msg == WM_CLIPBOARDUPDATE) {
            Write("clipboard.txt", Clipboard.ContainsText() ? Clipboard.GetText() : "");
        }
        base.WndProc(ref m);
    }

    public bool PreFilterMessage(ref Message m) {
        long w = m.WParam.ToInt64();
        long l = m.LParam.ToInt64();
        switch (m.Msg) {
            case 0x100: case 0x104: Key("down", w, l); break;
            case 0x101: case 0x105: Key("up", w, l); break;
            case 0x200: Pointer(m, "move", ((int)w & 0x73).ToString("x")); break;
            case 0x201: Pointer(m, "left-down", ""); break;
            case 0x202: Pointer(m, "left-up", ""); break;
            case 0x204: Pointer(m, "right-down", ""); break;
            case 0x205: Pointer(m, "right-up", ""); break;
            case 0x207: Pointer(m, "middle-down", ""); break;
            case 0x208: Pointer(m, "middle-up", ""); break;
            case 0x20B: Pointer(m, XButton(w) + "-down", ""); break;
            case 0x20C: Pointer(m, XButton(w) + "-up", ""); break;
            case 0x20A: Wheel("wheel", w, l); break;
            case 0x20E: Wheel("hwheel", w, l); break;
        }
        return false;
    }

    static string XButton(long w) {
        return ((w >> 16) & 0xFFFF) == 1 ? "x1" : "x2";
    }

    static void Key(string what, long w, long l) {
        long scancode = (l >> 16) & 0xFF;
        long extended = (l >> 24) & 1;
        Append("keys.txt", string.Format("{0} {1:x2} {2:x2} {3}", what, w & 0xFF, scancode, extended));
    }

    static void Pointer(Message m, string what, string detail) {
        long l = m.LParam.ToInt64();
        var at = new Point((short)(l & 0xFFFF), (short)((l >> 16) & 0xFFFF));
        var control = Control.FromHandle(m.HWnd);
        if (control != null) at = control.PointToScreen(at);
        Append("pointer.txt", string.Format("{0} {1} {2} {3}", what, at.X, at.Y, detail).TrimEnd());
    }

    // Wheel messages carry screen coordinates already.
    static void Wheel(string what, long w, long l) {
        int delta = (short)((w >> 16) & 0xFFFF);
        int x = (short)(l & 0xFFFF);
        int y = (short)((l >> 16) & 0xFFFF);
        Append("pointer.txt", string.Format("{0} {1} {2} {3}", what, x, y, delta));
    }

    // Polled with GetSystemMetrics: after a reconnect at another size, WM_DISPLAYCHANGE and
    // Screen.PrimaryScreen left the window at the old size.
    void FollowScreen() {
        var size = new Size(GetSystemMetrics(0), GetSystemMetrics(1));
        if (size == screen) return;
        screen = size;
        Bounds = new Rectangle(Point.Empty, size);
        Write("screen.txt", size.Width + " " + size.Height);
    }

    void Poll() {
        FollowScreen();
        var stamp = Stamp("set-clipboard.txt");
        if (stamp != clipboardFile) {
            clipboardFile = stamp;
            var text = Read("set-clipboard.txt");
            if (text.Length > 0) Clipboard.SetText(text);
        }
        stamp = Stamp("colour.txt");
        if (stamp != colourFile) {
            colourFile = stamp;
            var colour = Read("colour.txt").Trim();
            if (colour.Length == 6) BackColor = Color.FromArgb(Convert.ToInt32("ff" + colour, 16));
        }
        if (File.Exists(Dir + "logoff.txt")) {
            File.Delete(Dir + "logoff.txt");
            System.Diagnostics.Process.Start("logoff.exe");
        }
    }

    static DateTime Stamp(string name) {
        return File.GetLastWriteTimeUtc(Dir + name);
    }

    // The host reads these files at any moment, so a write can meet an open file.
    static void Retry(Action action) {
        for (int i = 0; ; i++) {
            try {
                action();
                return;
            } catch (IOException) {
                if (i == 20) throw;
                Thread.Sleep(25);
            }
        }
    }

    static string Read(string name) {
        string text = "";
        Retry(delegate { text = File.ReadAllText(Dir + name, Utf8); });
        return text;
    }

    static void Write(string name, string text) {
        Retry(delegate { File.WriteAllText(Dir + name, text, Utf8); });
    }

    static void Append(string name, string line) {
        Retry(delegate { File.AppendAllText(Dir + name, line + "\n", Utf8); });
    }
}
'@

[Probe]::Main()
