[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AllowedRoot,
    [Parameter(Mandatory)][string]$RunRoot,
    [Parameter(Mandatory)][ValidateSet('A','B')][string]$Key
)
$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot 'FixtureSafety.psm1') -Force
$State = Get-FixtureOwnershipState -AllowedRoot $AllowedRoot -RunRoot $RunRoot
$RunRoot = $State.runRoot
$Distribution = Get-WslDistributionRecord -DistroName $State.distroName
if (-not $Distribution) { throw 'Owned WSL distribution is not registered.' }
Assert-OwnedDistribution -State $State -Distribution $Distribution
& wsl.exe --terminate $State.distroName 2>$null
$Suffix = $Key.ToLowerInvariant()
& wsl.exe -d $State.distroName -u root -- sh -c "cp /opt/nexusops-fixture/host_ed25519_$Suffix /opt/nexusops-fixture/host_ed25519 && cp /opt/nexusops-fixture/host_ed25519_$Suffix.pub /opt/nexusops-fixture/host_ed25519.pub && chmod 600 /opt/nexusops-fixture/host_ed25519*"
if ($LASTEXITCODE -ne 0) { throw "Failed to activate host key $Key" }
& (Join-Path $PSScriptRoot "Start-OpenSshFixture.ps1") -AllowedRoot $AllowedRoot -RunRoot $RunRoot
