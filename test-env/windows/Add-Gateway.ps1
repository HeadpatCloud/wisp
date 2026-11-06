# Sets the VM wisp-test-rdp up as an RD Gateway for the gateway tests in
# src-tauri/tests/live_rdp.rs: the role, a self-signed certificate, a computer group, a CAP and a
# RAP. Each part that is in place already is left as it is. README.md lists what this does and how
# to undo it.
#   .\Add-Gateway.ps1
$ErrorActionPreference = 'Stop'
$password = ConvertTo-SecureString 'Wisp-Test-2026' -AsPlainText -Force
$credential = New-Object System.Management.Automation.PSCredential('wisp', $password)

Invoke-Command -VMName wisp-test-rdp -Credential $credential -ScriptBlock {
    $ErrorActionPreference = 'Stop'
    $changed = $false
    $name = $env:COMPUTERNAME
    $address = (Get-NetIPAddress -AddressFamily IPv4 -InterfaceAlias Ethernet).IPAddress

    $missing = Get-WindowsFeature RDS-Gateway, RSAT-RDS-Gateway | Where-Object { -not $_.Installed }
    if ($missing) {
        # About 4 minutes. It brings NPAS, IIS and the RPC-over-HTTP proxy along.
        $result = Install-WindowsFeature RDS-Gateway -IncludeManagementTools
        if (-not $result.Success) { throw "the role did not install: $($result.ExitCode)" }
        "role: installed, restart needed: $($result.RestartNeeded)"
        $changed = $true
    } else {
        'role: in place'
    }
    Import-Module RemoteDesktopServices

    $thumbprint = (Get-Item RDS:\GatewayServer\SSLCertificate\Thumbprint).CurrentValue
    $bound = $null
    if ($thumbprint) {
        $bound = Get-ChildItem Cert:\LocalMachine\My |
            Where-Object { $_.Thumbprint -eq $thumbprint -and $_.NotAfter -gt (Get-Date) }
    }
    if ($bound) {
        $until = $bound.NotAfter.ToString('yyyy-MM-dd')
        "certificate: in place, $thumbprint, $($bound.Subject), until $until"
    } else {
        $certificate = New-SelfSignedCertificate -Subject "CN=$name" `
            -TextExtension @("2.5.29.17={text}DNS=$name&IPAddress=$address") `
            -CertStoreLocation Cert:\LocalMachine\My -FriendlyName 'wisp-test RD Gateway' `
            -NotAfter (Get-Date).AddYears(2)
        Set-Item RDS:\GatewayServer\SSLCertificate\Thumbprint -Value $certificate.Thumbprint
        "certificate: made and bound, $($certificate.Thumbprint) for $name and $address"
        $changed = $true
    }

    $groups = 'RDS:\GatewayServer\GatewayManagedComputerGroups'
    if (Test-Path "$groups\WispTestComputers") {
        $computers = Get-ChildItem "$groups\WispTestComputers\Computers" | ForEach-Object Name
        "computer group: in place, $($computers -join ', ')"
    } else {
        New-Item -Path $groups -Name WispTestComputers -Description 'the VM itself' `
            -Computers @($name, $address, 'localhost', '127.0.0.1') | Out-Null
        "computer group: made, $name, $address, localhost, 127.0.0.1"
        $changed = $true
    }

    if (Test-Path RDS:\GatewayServer\CAP\WispCAP) {
        'CAP: in place'
    } else {
        # AuthMethod 1: a password.
        New-Item -Path RDS:\GatewayServer\CAP -Name WispCAP -UserGroups 'Administrators@BUILTIN' `
            -AuthMethod 1 | Out-Null
        'CAP: made, Administrators@BUILTIN with a password'
        $changed = $true
    }

    if (Test-Path RDS:\GatewayServer\RAP\WispRAP) {
        'RAP: in place'
    } else {
        New-Item -Path RDS:\GatewayServer\RAP -Name WispRAP -UserGroups 'Administrators@BUILTIN' `
            -ComputerGroupType 0 -ComputerGroup WispTestComputers | Out-Null
        'RAP: made, Administrators@BUILTIN to WispTestComputers'
        $changed = $true
    }
    $ports = (Get-Item RDS:\GatewayServer\RAP\WispRAP\PortNumbers).CurrentValue
    if ($ports -eq '3389') {
        'RAP port: in place, 3389'
    } else {
        Set-Item RDS:\GatewayServer\RAP\WispRAP\PortNumbers -Value 3389
        "RAP port: 3389, was $ports"
        $changed = $true
    }

    if ($changed) {
        # The gateway takes a new certificate only after a restart.
        Restart-Service TSGateway
        "gateway: restarted, $((Get-Service TSGateway).Status)"
    } else {
        "gateway: not restarted, $((Get-Service TSGateway).Status)"
    }
}
