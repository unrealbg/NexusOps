[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$RunRoot,
    [Parameter(Mandatory)][ValidateSet('A','B')][string]$Key
)
$ErrorActionPreference = "Stop"
$RunRoot = (Resolve-Path $RunRoot).Path
$State = Get-Content -Raw (Join-Path $RunRoot "fixture-owner.json") | ConvertFrom-Json
if ($State.marker -ne "nexusops-goal-02a-disposable-openssh" -or $State.runRoot -ne $RunRoot) {
    throw "Fixture ownership marker does not match"
}
& wsl.exe --terminate $State.distroName 2>$null
$Suffix = $Key.ToLowerInvariant()
& wsl.exe -d $State.distroName -u root -- sh -c "cp /opt/nexusops-fixture/host_ed25519_$Suffix /opt/nexusops-fixture/host_ed25519 && cp /opt/nexusops-fixture/host_ed25519_$Suffix.pub /opt/nexusops-fixture/host_ed25519.pub && chmod 600 /opt/nexusops-fixture/host_ed25519*"
if ($LASTEXITCODE -ne 0) { throw "Failed to activate host key $Key" }
& (Join-Path $PSScriptRoot "Start-OpenSshFixture.ps1") -RunRoot $RunRoot
