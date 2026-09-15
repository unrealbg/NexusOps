[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'FixtureSafety.psm1') -Force

$TestRoot = Join-Path ([IO.Path]::GetTempPath()) "nexusops-fixture-safety-$([Guid]::NewGuid().ToString('N'))"
$AllowedRoot = Join-Path $TestRoot 'work'
$OutsideRoot = Join-Path $TestRoot 'outside'
New-Item -ItemType Directory -Path $AllowedRoot, $OutsideRoot | Out-Null
$Calls = [Collections.Generic.List[string]]::new()
function Assert-Throws([scriptblock]$Action, [string]$Name) {
    try { & $Action; throw "Expected rejection: $Name" } catch {
        if ($_.Exception.Message -eq "Expected rejection: $Name") { throw }
    }
}

try {
    Assert-Throws { Assert-FixtureRunRoot -AllowedRoot $AllowedRoot -RunRoot $AllowedRoot } 'approved root itself'
    Assert-Throws { Assert-FixtureRunRoot -AllowedRoot $AllowedRoot -RunRoot $OutsideRoot } 'outside path'
    Assert-Throws { Assert-FixtureRunRoot -AllowedRoot $AllowedRoot -RunRoot "$AllowedRoot-other\fixture" } 'prefix collision'

    $Redirect = Join-Path $AllowedRoot 'redirect'
    New-Item -ItemType Junction -Path $Redirect -Target $OutsideRoot | Out-Null
    Assert-Throws { Assert-FixtureRunRoot -AllowedRoot $AllowedRoot -RunRoot (Join-Path $Redirect 'fixture') } 'reparse redirection'

    $RunRoot = Join-Path $AllowedRoot 'owned-fixture'
    $InstallDir = Join-Path $RunRoot 'distro'
    New-Item -ItemType Directory -Path $InstallDir | Out-Null
    $State = [pscustomobject]@{
        marker = 'nexusops-openssh-fixture-v2'
        ownershipId = [Guid]::NewGuid().ToString('D')
        allowedRoot = $AllowedRoot
        runRoot = $RunRoot
        installDir = $InstallDir
        distroName = 'NexusOps-OpenSSH-Test'
        distroRegistryId = 'owned-registry-id'
        status = 'ready'
    }
    $State | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $RunRoot 'fixture-owner.json') -Encoding utf8
    $Validated = Get-FixtureOwnershipState -AllowedRoot $AllowedRoot -RunRoot $RunRoot

    $WrongDistribution = [pscustomobject]@{ Name = $State.distroName; InstallDir = $OutsideRoot; RegistryId = 'other-id' }
    Assert-Throws {
        Invoke-OwnedFixtureCleanup -State $Validated -Distribution $WrongDistribution `
            -Terminate { param($Name) $Calls.Add("terminate:$Name") } `
            -Unregister { param($Name) $Calls.Add("unregister:$Name") } `
            -RemoveDirectory { param($Path) $Calls.Add("remove:$Path") }
    } 'reused distribution with another installation'
    if ($Calls.Count -ne 0) { throw 'A destructive operation ran for an unowned distribution.' }

    $OwnedDistribution = [pscustomobject]@{ Name = $State.distroName; InstallDir = $InstallDir; RegistryId = 'owned-registry-id' }
    Invoke-OwnedFixtureCleanup -State $Validated -Distribution $OwnedDistribution `
        -Terminate { param($Name) $Calls.Add("terminate:$Name") } `
        -Unregister { param($Name) $Calls.Add("unregister:$Name") } `
        -RemoveDirectory { param($Path) $Calls.Add("remove:$Path") }
    if ($Calls.Count -ne 3) { throw 'Legitimate owned cleanup did not invoke all mocked operations.' }

    $State.marker = 'stale-marker'
    $State | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $RunRoot 'fixture-owner.json') -Encoding utf8
    Assert-Throws { Get-FixtureOwnershipState -AllowedRoot $AllowedRoot -RunRoot $RunRoot } 'invalid or stale marker'
    'Fixture safety tests passed: 7 cases.'
} finally {
    Remove-Item -LiteralPath $TestRoot -Recurse -Force
}
