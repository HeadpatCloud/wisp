# Windows Server test VM

A Hyper-V VM with Windows Server 2025 for the Windows tests in `src-tauri/tests/live_rdp.rs`.
It is reachable from this host and from other VMs on its Default Switch (Hyper-V's
`Default Switch`). The passwords below are fixed and written down here and account lockout is
off, so the VM must never be connected to another network. Through the Default Switch's NAT it
does reach the internet: it has automatic updates off, a fixed password and no lockout.

- VM `wisp-test-rdp`, generation 2, 4 GB RAM (static), 2 virtual processors, a 40 GB dynamic
  disk, on the existing `Default Switch`, automatic checkpoints off.
- Windows Server 2025 Standard Evaluation (Desktop Experience), image 2 of the evaluation ISO
  `26100.1742.240906-0331.ge_release_svc_refresh_SERVER_EVAL_x64FRE_en-us.iso`. The evaluation
  runs for 180 days.
- Computer name `WISP-TEST-RDP`, time zone UTC, en-US with the US keyboard.
- Local administrator `wisp`, password `Wisp-Test-2026`. The built-in `Administrator` gets the
  same password, as setup asks for one otherwise.

## Create and remove

From PowerShell on the host, in this folder. Neither script needs an elevated shell; the user
has to be in `Hyper-V Administrators`.

```powershell
powershell -ExecutionPolicy Bypass -File .\New-TestVm.ps1               # about 5 minutes
powershell -ExecutionPolicy Bypass -File .\New-TestVm.ps1 -Configure    # after probe.ps1 changed
powershell -ExecutionPolicy Bypass -File .\Remove-TestVm.ps1
```

Both scripts take `-Root <folder>` for another place than `F:\wisp-test`: the VM's folder is
`<folder>\wisp-test-rdp`, and the ISO is looked for in `<folder>` unless `New-TestVm.ps1 -Iso
<path>` names another place for it. The VM's address comes from Hyper-V:
`(Get-VMNetworkAdapter -VMName wisp-test-rdp).IPAddresses`. When `New-TestVm.ps1` fails, run
`Remove-TestVm.ps1` before running it again: it stops when the VM or its folder is already
there.

### What New-TestVm.ps1 does

On the host, and nothing else:

1. Creates `F:\wisp-test\wisp-test-rdp\` and in it `answer.iso`, an ISO 9660 + Joliet image with
   `autounattend.xml` at its root, built with the IMAPI2 COM objects that come with Windows
   (`IMAPI2FS.MsftFileSystemImage`).
2. `New-VM` with the disk `F:\wisp-test\wisp-test-rdp\wisp-test-rdp.vhdx` and the VM's
   configuration under `F:\wisp-test\wisp-test-rdp\`; static memory, 2 processors, automatic
   checkpoints off; two DVD drives (the Windows ISO, then `answer.iso`); first boot device the
   Windows DVD.
3. Starts the VM and types Enter into it 20 times, every 0.5 s, through Hyper-V's
   `Msvm_Keyboard.TypeKey` (see the boot prompt below).
4. Waits until PowerShell Direct (`Invoke-Command -VMName`) can log in as `wisp`: setup is done.
5. Inside the VM through PowerShell Direct (the `-Configure` part):
   - `C:\wisp\` with `probe.ps1` in it; `wisp` gets Modify on the folder and everything in it;
   - Remote Desktop on (`fDenyTSConnections` 0), Network Level Authentication required
     (`UserAuthentication` 1 on `RDP-Tcp`), the firewall rules of the group `Remote Desktop` on;
   - the scheduled task `WispProbe`, which starts `probe.ps1` in the session at every logon of
     `wisp` (interactive, not elevated);
   - Server Manager does not open at logon (`DoNotOpenServerManagerAtLogon` and its scheduled
     task disabled), no first-logon animation, no question why the computer shut down
     (`ShutdownReasonOn` policy 0);
   - automatic updates off (`NoAutoUpdate` policy), so that no update restarts the VM during a
     test run;
   - account lockout off (`net accounts /lockoutthreshold:0`): the tests try wrong passwords;
   - no recovery screen at boot (`bcdedit` `bootstatuspolicy ignoreallfailures`,
     `recoveryenabled no`): a test turns the VM off.
6. Restarts the VM and prints its address.

`Remove-TestVm.ps1` turns the VM off, removes it from Hyper-V and deletes
`F:\wisp-test\wisp-test-rdp\`. The ISO stays in `F:\wisp-test\`. No other host setting is
changed by either script: no firewall, network, registry or service change on the host.

### How it went (2026-10-21)

- Generation 2 worked at the first try; generation 1 was not tried. The DVD's boot loader shows
  `Press any key to boot from CD or DVD...` for a few seconds and starts setup only after a key,
  so the script types Enter into the VM (starting without the key was not tried). After setup's
  restarts nobody presses a key, the prompt runs out and the VM boots from the disk.
- Setup finds `autounattend.xml` on the second DVD by itself.
- From `Start-VM` to PowerShell Direct answering: under 4 minutes; the whole script, the
  restart included, 3 min 58 s to 4 min 7 s (the last three of four runs; 32 logical
  processors on the host, the VM's disk on F:). `Remove-TestVm.ps1` took 1.5 s.
- `Test-NetConnection <address> -Port 3389` succeeded right after. The address comes from the
  `Default Switch`'s DHCP and was different for each new VM (172.31.139.153, 172.31.129.217,
  172.31.141.52, 172.31.133.188); a turn-off and start kept it.
- The first version restarted the VM with `Restart-VM -Force`, which resets it: the firewall
  rules and the policy values that the setup had written just before were gone after the
  restart (the scheduled task was not). `Restart-VM -Type Reboot` keeps them.
- PowerShell Direct runs elevated, the probe does not. Files the host wrote into `C:\wisp`
  belonged to Administrators, and the probe could not delete them (`Access to the path
  'C:\wisp\logoff.txt' is denied`, shown in a WinForms error box in the session). The script
  therefore gives `wisp` Modify on the folder.

## The probe

`probe.ps1` runs at every logon of `wisp` (a new session; a reconnect to a disconnected session
keeps the one that runs). It shows a borderless, topmost window over the whole screen:
background `306090`, a white text box at 20,20 (600x120) that has the focus, and four bars of
160x80 at 20,160: `ff0000`, `00ff00`, `0000ff`, `ffff00`. It writes into `C:\wisp\` (UTF-8
without BOM, lines end in LF):

| file | content |
|---|---|
| `typed.txt` | the text box's text, on every change (a RichTextBox: Return is LF) |
| `keys.txt` | one line per key message: `down` or `up`, virtual key, scancode (hex), extended bit |
| `pointer.txt` | `move x y buttons`, `left-down x y`, `left-up`, `right-`, `middle-`, `x1-`, `x2-`, `wheel x y delta`, `hwheel x y delta` (screen coordinates, buttons as `MK_` bits in hex) |
| `clipboard.txt` | the clipboard's text whenever the clipboard changes |
| `screen.txt` | the screen size, `width height`, at start and when it changes |
| `layout.txt` | the keyboard layout of the session (language id in hex, e.g. `0409`) |

And it acts on files the host writes:

| file | what the probe does |
|---|---|
| `set-clipboard.txt` | puts its text on the clipboard when the file changes |
| `colour.txt` | a colour `rrggbb` for the window's background, when the file changes |
| `logoff.txt` | deletes it and runs `logoff.exe`, when it appears |

The host reads and writes these files with PowerShell Direct, never through the RDP session:

```powershell
$credential = Get-Credential wisp     # Wisp-Test-2026
Invoke-Command -VMName wisp-test-rdp -Credential $credential { Get-Content C:\wisp\typed.txt }
```

## What was measured (2026-10-21, with the app's client)

- Security: TLS with CredSSP (NLA). The certificate is the self-signed one of the RDP listener
  (`Cert:\LocalMachine\Remote Desktop`, the one `Win32_TSGeneralSetting.SSLCertificateSHA1Hash`
  names); the fingerprint the client shows is the SHA-256 of its DER form.
- Sign-in refusals: Windows answers a wrong password, an unknown user, a wrong password for
  `Administrator` and an empty password all with `STATUS_LOGON_FAILURE` (`0xc000006d`) in
  CredSSP. The app does not send an empty password (`this server needs a user name and
  password`); `0xc000006d` for it was measured with that check taken out. An unknown domain
  (`OTHERDOMAIN` in the domain field, or `OTHER\wisp`) logs in as the local `wisp`.
- `WISP-TEST-RDP\wisp` and `wisp` with the domain `WISP-TEST-RDP` log in. `wisp@WISP-TEST-RDP`
  is refused as a wrong user name or password: NTLM carries a UPN whole in the user name and
  leaves the domain empty, and a local account has no UPN.
- The session gets the size the client asks for (1280x800, 1440x900). A connection to the
  disconnected session of `wisp` takes that session back at the new size. Windows sets the
  keyboard layout at logon (`0x407` gives the German layout) and a later connection with another
  layout keeps the session's.
- No PDU the app counts as unhandled (`Session::skipped`) arrived in any test.
- Windows merges pointer messages that wait in the probe's queue: of two moves sent at once only
  the last arrives, and wheel up and wheel down sent at once arrive as one `wheel` of 0. The
  tests wait for each move and each button to show before they send the next.
- `logoff.exe` run by the probe in a connected session logs off; run in a disconnected session it
  does nothing. The tests log a session off from outside with `logoff <session id>`.
- Clipboard: Windows announces `CF_UNICODETEXT`, `CF_LOCALE`, `CF_TEXT` and `CF_OEMTEXT`.
- A session closed the moment `connect` returned (0 ms; 20 ms and more did not) left the
  session's `rdpclip.exe` stuck: no later connection to that session got a clipboard channel
  (no Monitor Ready), 3 times out of 3 from a working one. Ending that `rdpclip.exe` by its id
  through PowerShell Direct brings it back: the next connection starts a new one; so does
  logging the session off. Whether Microsoft's own client does the same was not tried. The
  tests wait for the picture before they close.
- That is Windows, not the client (2026-10-22, every PDU of the client traced): closing a
  connection to the disconnected session 0 ms after `connect` left `rdpclip.exe` stuck in 4 of
  11 tries, closing it after 20 ms in none of 5, and a 0 ms close of a new logon did not (1
  try). In the 1 to 2 ms before such a close Windows sends nothing on the clipboard channel (its
  Capabilities and Monitor Ready come 65 to 290 ms after `connect` for a disconnected session,
  0.9 to 1.2 s for a new logon), so there is no clipboard message the client could leave
  unanswered. The client answers the dynamic channel capabilities and the channel creates that
  have arrived (up to six), then sends the Shutdown Request, and Windows closes the connection
  without a Shutdown Request Denied. In 3 of the 4 stuck tries the client sent the same 8 PDUs in
  the same order as in 4 of the 5 20 ms tries (the fifth got one more channel create).
- `close()`: the client sends the Shutdown Request, Windows answers with Shutdown Request Denied
  within a millisecond, the client sends the MCS Disconnect Provider Ultimatum and the session
  ends: about 2 ms.
- `Stop-VM -TurnOff` with a session up: nothing comes back, so only TCP keepalive notices.
  Windows' default of 10 probes made that 130 s (30 s idle + 10 x 10 s); the client now asks for
  2 probes: 50 s. `Start-VM` brings RDP and PowerShell Direct back in about 10 s. The price:
  a session with nothing to send now ends when the link is down for about 20 s or more (the two
  probes 10 s apart), and a dead link still takes about 50 s to notice.
- After a few `Stop-VM -TurnOff` in a row Windows started into its `Recovery` screen
  (`It looks like Windows didn't load correctly`) and waited there; the `bcdedit` settings above
  stop that.
- The first logon after a turn-off showed the Shutdown Event Tracker (`shutdown.exe -unexpected`
  in the session), and the probe was not running in that session; the policy above stops it.
- The probe was first started by an `HKLM` Run value. After a reboot Explorer ran it for two
  logons and then, for every later logon, only `RunOnce` (Shell-Core events 9705/9706); the
  scheduled task started it at every logon of the test runs since.
- In the first session after setup `dwm.exe` crashed once in `dwmredir.dll` (Application log,
  0x80070490); nothing was seen of it from the client.

## RD Gateway

`Add-Gateway.ps1` makes the VM an RD Gateway for the gateway tests. From PowerShell on the host,
in this folder, with the VM running:

```powershell
powershell -ExecutionPolicy Bypass -File .\Add-Gateway.ps1
```

Inside the VM through PowerShell Direct, each part only when it is not there yet:

- the role `RDS-Gateway` with its management tools (`Install-WindowsFeature RDS-Gateway
  -IncludeManagementTools`, about 4 minutes; it brings NPAS, IIS with Basic and Windows
  authentication, the RPC-over-HTTP proxy and the RSAT parts along);
- a self-signed certificate `CN=WISP-TEST-RDP` for the computer name and the VM's address, valid
  for 2 years, in `Cert:\LocalMachine\My`, bound to the gateway
  (`RDS:\GatewayServer\SSLCertificate\Thumbprint`); a bound certificate that has not expired is
  kept;
- the gateway-managed computer group `WispTestComputers`: the computer name, the VM's address,
  `localhost`, `127.0.0.1`;
- the CAP `WispCAP` (`Administrators@BUILTIN`, password logon) and the RAP `WispRAP`
  (`Administrators@BUILTIN` to `WispTestComputers`, port 3389);
- a restart of the `TSGateway` service when anything changed, as the gateway takes a new
  certificate only then.

The gateway listens on port 443 of the VM only, so a gateway on another port cannot be tried
here. The role's firewall rule `Remote Desktop Gateway HTTP Listener` stays off, and 443 is
reachable from the host anyway. It takes HTTP Basic as it is installed.

The computer group holds the address the VM had when the script made it, and the Default Switch
hands out another one after the host restarts (172.23.107.46 when the group was made on
2026-10-28, 172.23.80.222 on 2026-11-06). The tests therefore reach the RDP server through the
gateway by the computer name, which does not change. The certificate keeps the old address too;
the client pins it (`rdg/<address>` at port 443), so that does not matter to the tests.

On 2026-11-06 the script found everything in place and changed nothing:

```
role: in place
certificate: in place, BB64D137FDD4AA27B704F52677E0093DD92C8E8E, CN=WISP-TEST-RDP, until 2028-10-28
computer group: in place, 127.0.0.1, 172.23.107.46, localhost, WISP-TEST-RDP
CAP: in place
RAP: in place
RAP port: in place, 3389
gateway: not restarted, Running
```

To undo it, inside the VM (PowerShell Direct, as `wisp`):

```powershell
Import-Module RemoteDesktopServices
Remove-Item RDS:\GatewayServer\RAP\WispRAP -Recurse
Remove-Item RDS:\GatewayServer\CAP\WispCAP -Recurse
Remove-Item RDS:\GatewayServer\GatewayManagedComputerGroups\WispTestComputers -Recurse
Uninstall-WindowsFeature RDS-Gateway,NPAS,Web-Server,RPC-over-HTTP-Proxy -IncludeManagementTools
Remove-Item Cert:\LocalMachine\My\<thumbprint>     # restart first if the uninstall asks for it
```

## Scripts in the session

`\\tsclient` and the session's sound exist only inside the RDP session; PowerShell Direct runs in
a session of its own. `Invoke-InSession.ps1` runs a script inside `wisp`'s logged-on session and
prints what it printed:

```powershell
powershell -ExecutionPolicy Bypass -File .\Invoke-InSession.ps1 -Script 'Get-ChildItem C:\'
```

It writes the script to `C:\wisp\insession\`, registers the scheduled task `WispInSession` when
it is missing (`wisp`, interactive, not elevated, at most one at a time), starts it and waits
for `C:\wisp\insession\out.txt`, which the task writes once the script is done (`-Wait`, 60 s
unless given). It fails when `wisp` has no session. The live tests pass their PowerShell Direct
session with `-Session`. To remove the task and the folder:

```powershell
Unregister-ScheduledTask -TaskName WispInSession -Confirm:$false
Remove-Item -Recurse C:\wisp\insession
```

## What was measured for sound, the shared folder and the gateway (2026-11-06)

- Sound: nothing in the VM had to change (the Windows Audio service runs, redirection is on).
  Windows sends rdpsnd only to a client that also has an rdpdr channel: without one the session's
  `Remote Audio` device takes 5.4 s to open (`waveOutOpen`), a 2 s sound takes 7.5 s to play, and
  nothing comes; the client therefore always attaches rdpdr with sound on, without drives when no
  folder is shared. With it, 2 s of silence at 44100 Hz, 2 channels, 16 bit arrive as 11 Wave2
  blocks of 32768 bytes (360,448 bytes), directly and through the gateway.
- The shared folder: patterns reach the client as typed (`\a?c.txt`, `\*.txt`, `\a??.txt`,
  `\*.t?t`, `\*.`), never with the DOS wildcards `<`, `>` or `"`; `*.*` arrives as `*`. The client
  matches them as FindFirstFile does in a local NTFS folder, so `dir *.` lists the names without a
  dot there too.
- `..\..\Windows\win.ini` arrives as `\Windows\win.ini`; `readme.txt:ads` and every path through a
  junction to a folder beside the share are refused with STATUS_ACCESS_DENIED, which PowerShell
  shows as `Access is denied`.
- `FileStream.Flush($true)`, `Get-Acl` and `icacls` never reach the client: Windows answers them
  itself (owner `Everyone`, `No permissions are set. All users have full control.`).
  `FileStream.Lock()` does, gets STATUS_NOT_SUPPORTED and fails with `The request is not
  supported.`; a second open of a file that is open reads it.
- Gateway: a wrong gateway password gets a 401 that offers Basic; a server outside the RAP's
  computer group (192.0.2.1) is refused with 0x800759da (E_PROXY_RAP_ACCESSDENIED), event 301 in
  the gateway's log. A copy of 100,000 bytes from the share is one read of 100,000 bytes, written
  back through the tunnel in packets of at most 8 KB. In a session of 5 minutes through it (2.5
  minutes without input, then a key every 30 s) the gateway sent no WebSocket ping or pong and no
  MS-TSGU keepalive.

## Live tests

With the VM running (and the xrdp container up for the xrdp tests):

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test live_rdp -- --ignored --test-threads=1
```

A Windows test that does not find the VM running prints why and passes. The two gateway tests,
`windows_through_the_gateway` and `windows_gateway_names_each_refusal`, also need
`Add-Gateway.ps1` run once on a VM that `New-TestVm.ps1` made. The tests keep one
PowerShell process with one PowerShell Direct session for the whole run. One test turns the VM
off and starts it again; one logs the session off; the German layout test logs the session off
first when the probe reports another layout, and the next test that wants the US layout does
the same. The VM is left running.

The last test, `windows_what_applications_do_on_the_share_works`, logs off the session `wisp`
was left in. A filtered run that skips it leaves a disconnected `wisp` session in the VM; to log
it off:

```powershell
$credential = Get-Credential wisp     # Wisp-Test-2026
Invoke-Command -VMName wisp-test-rdp -Credential $credential {
    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { logoff $_.SessionId } }
```
