# Runs a PowerShell script inside the logged-on session of wisp in the VM wisp-test-rdp and prints
# what it printed. \\tsclient and the session's sound are only there: PowerShell Direct runs in a
# session of its own. A scheduled task in wisp's session runs the script.
#   .\Invoke-InSession.ps1 -Script 'Get-ChildItem \\tsclient\wisp'
param(
    [Parameter(Mandatory)][string]$Script,
    [int]$Wait = 60,
    # An open PowerShell Direct session to the VM as wisp, instead of a new one.
    [System.Management.Automation.Runspaces.PSSession]$Session
)

$ErrorActionPreference = 'Stop'
$block = {
    param($text, $wait)
    $ErrorActionPreference = 'Stop'
    $folder = 'C:\wisp\insession'
    $out = "$folder\out.txt"
    # quser writes to stderr when nobody is logged on, which 'Stop' would turn into an error.
    if (-not ((cmd /c 'quser 2>nul') -match '^\s*>?wisp\s')) { throw 'wisp has no session' }
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    # out.txt appears only once the script is done, whole, so the side waiting for it never reads
    # it half written or holds it while the task writes.
    Set-Content -Encoding UTF8 "$folder\run.ps1" @'
$printed = try {
    & C:\wisp\insession\script.ps1 *>&1 | ForEach-Object { "$_" }
} catch {
    "error: $_"
}
Set-Content -Encoding UTF8 C:\wisp\insession\out.tmp $printed
Move-Item -Force C:\wisp\insession\out.tmp C:\wisp\insession\out.txt
'@
    Set-Content -Encoding UTF8 "$folder\script.ps1" $text
    if (-not (Get-ScheduledTask -TaskName WispInSession -ErrorAction SilentlyContinue)) {
        $arguments = "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File $folder\run.ps1"
        $action = New-ScheduledTaskAction -Execute powershell.exe -Argument $arguments
        $principal = New-ScheduledTaskPrincipal -UserId wisp -LogonType Interactive `
            -RunLevel Limited
        $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries `
            -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew `
            -ExecutionTimeLimit (New-TimeSpan -Minutes 10)
        Register-ScheduledTask -TaskName WispInSession -Action $action -Principal $principal `
            -Settings $settings | Out-Null
    }
    if (Test-Path $out) { Remove-Item $out }
    Start-ScheduledTask -TaskName WispInSession
    $until = (Get-Date).AddSeconds($wait)
    while (-not (Test-Path $out)) {
        if ((Get-Date) -gt $until) {
            $state = (Get-ScheduledTask -TaskName WispInSession).State
            $result = '0x{0:x8}' -f (Get-ScheduledTaskInfo -TaskName WispInSession).LastTaskResult
            # A task still running would make the next start a no-op, and its late out.txt the
            # next call's answer.
            Stop-ScheduledTask -TaskName WispInSession
            throw "the script did not end within $wait s (task $state, last result $result)"
        }
        Start-Sleep -Milliseconds 200
    }
    # A scan of the file just moved there can hold it for a moment.
    $tries = 0
    while ($true) {
        try {
            $printed = Get-Content -Raw -Encoding UTF8 $out
            break
        } catch {
            $tries += 1
            if ($tries -ge 5) { throw }
            Start-Sleep -Milliseconds 200
        }
    }
    if ($printed) { $printed.TrimEnd("`r", "`n") -split "`r?`n" }
}

if ($Session) {
    Invoke-Command -Session $Session -ScriptBlock $block -ArgumentList $Script, $Wait
} else {
    $password = ConvertTo-SecureString 'Wisp-Test-2026' -AsPlainText -Force
    $credential = New-Object System.Management.Automation.PSCredential('wisp', $password)
    Invoke-Command -VMName wisp-test-rdp -Credential $credential -ScriptBlock $block `
        -ArgumentList $Script, $Wait
}
