[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AllowedRoot,
    [Parameter(Mandatory)][string]$RunRoot
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'FixtureSafety.psm1') -Force
$State = Get-FixtureOwnershipState -AllowedRoot $AllowedRoot -RunRoot $RunRoot
$Distribution = Get-WslDistributionRecord -DistroName $State.distroName

Invoke-OwnedFixtureCleanup -State $State -Distribution $Distribution `
    -Terminate {
        param($Name)
        & wsl.exe --terminate $Name 2>$null
    } `
    -Unregister {
        param($Name)
        & wsl.exe --unregister $Name
        if ($LASTEXITCODE -ne 0) { throw "Failed to unregister $Name" }
    } `
    -RemoveDirectory {
        param($Path)
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
