# Creates the Hyper-V VM wisp-test-rdp, installs Windows Server 2025 into it without questions and
# sets it up for src-tauri/tests/live_rdp.rs. README.md lists everything this does.
#   .\New-TestVm.ps1               the whole VM, about 5 minutes
#   .\New-TestVm.ps1 -Configure    only the setup inside a VM that is running, after probe.ps1 changed
param(
    [string]$Root = 'F:\wisp-test',
    [string]$Iso = (Join-Path $Root '26100.1742.240906-0331.ge_release_svc_refresh_SERVER_EVAL_x64FRE_en-us.iso'),
    [switch]$Configure
)

$ErrorActionPreference = 'Stop'
$name = 'wisp-test-rdp'
$folder = Join-Path $Root $name
$password = ConvertTo-SecureString 'Wisp-Test-2026' -AsPlainText -Force
$credential = New-Object System.Management.Automation.PSCredential('wisp', $password)

function New-AnswerIso([string]$path) {
    $staging = Join-Path $folder 'answer'
    New-Item -ItemType Directory -Path $staging | Out-Null
    Copy-Item (Join-Path $PSScriptRoot 'autounattend.xml') $staging
    $image = New-Object -ComObject IMAPI2FS.MsftFileSystemImage
    # ISO 9660 and Joliet.
    $image.FileSystemsToCreate = 3
    $image.VolumeName = 'WISPANSWER'
    $image.Root.AddTree($staging, $false)
    $stream = $image.CreateResultImage().ImageStream
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
public static class StreamFile {
    public static void Save(object source, string path) {
        var stream = (IStream)source;
        var buffer = new byte[65536];
        var read = Marshal.AllocHGlobal(4);
        try {
            using (var file = File.Create(path)) {
                while (true) {
                    stream.Read(buffer, buffer.Length, read);
                    var count = Marshal.ReadInt32(read);
                    if (count == 0) break;
                    file.Write(buffer, 0, count);
                }
            }
        } finally {
            Marshal.FreeHGlobal(read);
        }
    }
}
'@
    [StreamFile]::Save($stream, $path)
    Remove-Item -Recurse $staging
}

function Wait-Guest([TimeSpan]$limit) {
    $until = (Get-Date) + $limit
    while ($true) {
        try {
            Invoke-Command -VMName $name -Credential $credential -ScriptBlock { $true } | Out-Null
            return
        } catch {
            # Not up yet: setup is still running, or the account does not exist yet.
            if ((Get-Date) -gt $until) { throw "the guest did not answer within $limit" }
            Start-Sleep -Seconds 15
        }
    }
}

function Set-Guest {
    $session = New-PSSession -VMName $name -Credential $credential
    Invoke-Command -Session $session -ScriptBlock {
        $ErrorActionPreference = 'Stop'
        New-Item -ItemType Directory -Force -Path C:\wisp | Out-Null
        # PowerShell Direct runs elevated, the probe does not: files the host makes belong to
        # Administrators, and the probe has to be able to replace and delete them.
        icacls C:\wisp /grant 'wisp:(OI)(CI)M' /T | Out-Null
    }
    Copy-Item -ToSession $session -Path (Join-Path $PSScriptRoot 'probe.ps1') -Destination C:\wisp\
    Invoke-Command -Session $session -ScriptBlock {
        $ErrorActionPreference = 'Stop'
        $ts = 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server'
        Set-ItemProperty $ts -Name fDenyTSConnections -Value 0
        Set-ItemProperty "$ts\WinStations\RDP-Tcp" -Name UserAuthentication -Value 1
        Enable-NetFirewallRule -DisplayGroup 'Remote Desktop'

        # A task and not the Run key: after a few logons Explorer stopped running the Run key.
        $arguments = '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\wisp\probe.ps1'
        $action = New-ScheduledTaskAction -Execute powershell.exe -Argument $arguments
        $trigger = New-ScheduledTaskTrigger -AtLogOn -User wisp
        $principal = New-ScheduledTaskPrincipal -UserId wisp -LogonType Interactive -RunLevel Limited
        $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) `
            -MultipleInstances Parallel
        Register-ScheduledTask -TaskName WispProbe -Action $action -Trigger $trigger `
            -Principal $principal -Settings $settings -Force | Out-Null

        # Anything that opens a window at logon would cover the probe.
        $manager = 'HKLM:\SOFTWARE\Microsoft\ServerManager'
        Set-ItemProperty $manager -Name DoNotOpenServerManagerAtLogon -Value 1
        $tasks = '\Microsoft\Windows\Server Manager\'
        Disable-ScheduledTask -TaskPath $tasks -TaskName ServerManager | Out-Null
        $system = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
        Set-ItemProperty $system -Name EnableFirstLogonAnimation -Value 0
        # After a turn-off the next logon asks why the computer shut down, and the probe did not
        # start in that session.
        $reliability = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows NT\Reliability'
        New-Item -Force -Path $reliability | Out-Null
        Set-ItemProperty $reliability -Name ShutdownReasonOn -Value 0

        # An update that restarts the VM in the middle of a test run ends every session.
        $au = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU'
        New-Item -Force -Path $au | Out-Null
        Set-ItemProperty $au -Name NoAutoUpdate -Value 1

        # The tests try wrong passwords on purpose, and a locked account would stop every later
        # login.
        net accounts /lockoutthreshold:0 | Out-Null

        # A test turns the VM off; after a few of those Windows started into its recovery
        # screen and waited there.
        bcdedit /set '{current}' bootstatuspolicy ignoreallfailures | Out-Null
        bcdedit /set '{current}' recoveryenabled no | Out-Null
    }
    Remove-PSSession $session
}

if ($Configure) {
    Set-Guest
    return
}

if (Get-VM -Name $name -ErrorAction SilentlyContinue) { throw "$name exists already" }
if (-not (Test-Path $Iso)) { throw "$Iso is missing" }
New-Item -ItemType Directory -Path $folder | Out-Null
$answer = Join-Path $folder 'answer.iso'
New-AnswerIso $answer

$started = Get-Date
$vm = New-VM -Name $name -Generation 2 -MemoryStartupBytes 4GB -Path $Root `
    -NewVHDPath (Join-Path $folder "$name.vhdx") -NewVHDSizeBytes 40GB -SwitchName 'Default Switch'
Set-VMMemory -VM $vm -DynamicMemoryEnabled $false
Set-VMProcessor -VM $vm -Count 2
Set-VM -VM $vm -AutomaticCheckpointsEnabled $false
$dvd = Add-VMDvdDrive -VM $vm -Path $Iso -Passthru
Add-VMDvdDrive -VM $vm -Path $answer
Set-VMFirmware -VM $vm -FirstBootDevice $dvd
Start-VM -VM $vm

# The DVD's boot loader waits a few seconds for a key before it boots from the DVD.
$system = Get-CimInstance -Namespace root\virtualization\v2 -ClassName Msvm_ComputerSystem `
    -Filter "ElementName='$name'"
$keyboard = Get-CimAssociatedInstance -InputObject $system -ResultClassName Msvm_Keyboard
for ($i = 0; $i -lt 20; $i++) {
    Start-Sleep -Milliseconds 500
    Invoke-CimMethod -InputObject $keyboard -MethodName TypeKey -Arguments @{ keyCode = [uint32]0x0D } | Out-Null
}

Wait-Guest ([TimeSpan]::FromMinutes(90))
Write-Host "setup finished after $([int]((Get-Date) - $started).TotalMinutes) minutes"
Set-Guest
# A clean restart: a reset (the default) lost registry changes Set-Guest had just made.
Restart-VM -VM $vm -Type Reboot -Force -Wait -For Heartbeat
Wait-Guest ([TimeSpan]::FromMinutes(10))

$address = (Get-VMNetworkAdapter -VMName $name).IPAddresses | Where-Object { $_ -match '^\d+\.' } | Select-Object -First 1
Write-Host "$name is up at $address after $([int]((Get-Date) - $started).TotalMinutes) minutes"
