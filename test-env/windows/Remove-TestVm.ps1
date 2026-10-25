# Removes the VM wisp-test-rdp and its folder wisp-test-rdp under -Root (F:\wisp-test unless
# given). The ISO stays.
param(
    [string]$Root = 'F:\wisp-test'
)

$ErrorActionPreference = 'Stop'
$name = 'wisp-test-rdp'
$folder = Join-Path $Root $name

if (Get-VM -Name $name -ErrorAction SilentlyContinue) {
    Stop-VM -Name $name -TurnOff -Force
    Remove-VM -Name $name -Force
}
if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
