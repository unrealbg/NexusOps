Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:OwnershipMarker = 'nexusops-openssh-fixture-v2'

function Get-CanonicalPath {
    param([Parameter(Mandatory)][string]$Path, [switch]$RequireExisting)
    $Full = [IO.Path]::GetFullPath($Path)
    if ($RequireExisting -or (Test-Path -LiteralPath $Full)) {
        return (Resolve-Path -LiteralPath $Full -ErrorAction Stop).Path
    }
    return $Full.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
}

function Test-StrictChildPath {
    param([Parameter(Mandatory)][string]$Parent, [Parameter(Mandatory)][string]$Child)
    $Relative = [IO.Path]::GetRelativePath($Parent, $Child)
    if ($Relative -eq '.' -or $Relative -eq '..' -or [IO.Path]::IsPathRooted($Relative)) { return $false }
    return -not ($Relative.StartsWith("..$([IO.Path]::DirectorySeparatorChar)", [StringComparison]::Ordinal) -or
        $Relative.StartsWith("..$([IO.Path]::AltDirectorySeparatorChar)", [StringComparison]::Ordinal))
}

function Assert-NoReparsePoint {
    param([Parameter(Mandatory)][string]$AllowedRoot, [Parameter(Mandatory)][string]$RunRoot)
    $Current = $RunRoot
    while ($true) {
        if (Test-Path -LiteralPath $Current) {
            $Item = Get-Item -LiteralPath $Current -Force
            if (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Fixture paths must not contain reparse points: $Current"
            }
        }
        if ([string]::Equals($Current, $AllowedRoot, [StringComparison]::OrdinalIgnoreCase)) { break }
        $Parent = [IO.Path]::GetDirectoryName($Current)
        if (-not $Parent -or [string]::Equals($Parent, $Current, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Fixture path traversal did not reach the approved root.'
        }
        $Current = $Parent.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    }
}

function Assert-FixtureRunRoot {
    param(
        [Parameter(Mandatory)][string]$AllowedRoot,
        [Parameter(Mandatory)][string]$RunRoot,
        [switch]$RequireExisting
    )
    $Approved = Get-CanonicalPath -Path $AllowedRoot -RequireExisting
    if (-not (Get-Item -LiteralPath $Approved).PSIsContainer) { throw 'AllowedRoot must be a directory.' }
    $Owned = Get-CanonicalPath -Path $RunRoot -RequireExisting:$RequireExisting
    if (-not (Test-StrictChildPath -Parent $Approved -Child $Owned)) {
        throw "RunRoot must be a dedicated child of the approved root: $Approved"
    }
    Assert-NoReparsePoint -AllowedRoot $Approved -RunRoot $Owned
    [pscustomobject]@{ AllowedRoot = $Approved; RunRoot = $Owned }
}

function Get-FixtureOwnershipState {
    param(
        [Parameter(Mandatory)][string]$AllowedRoot,
        [Parameter(Mandatory)][string]$RunRoot,
        [switch]$AllowImporting
    )
    $Paths = Assert-FixtureRunRoot -AllowedRoot $AllowedRoot -RunRoot $RunRoot -RequireExisting
    $MarkerPath = Join-Path $Paths.RunRoot 'fixture-owner.json'
    if (-not (Test-Path -LiteralPath $MarkerPath -PathType Leaf)) { throw 'Fixture ownership marker is missing.' }
    $State = Get-Content -LiteralPath $MarkerPath -Raw | ConvertFrom-Json
    foreach ($Name in @('marker', 'ownershipId', 'allowedRoot', 'runRoot', 'installDir', 'distroName', 'status')) {
        if (-not $State.PSObject.Properties[$Name] -or [string]::IsNullOrWhiteSpace([string]$State.$Name)) {
            throw "Fixture ownership marker is missing $Name."
        }
    }
    $OwnershipId = [Guid]::Empty
    if (-not [Guid]::TryParse([string]$State.ownershipId, [ref]$OwnershipId) -or $OwnershipId -eq [Guid]::Empty) {
        throw 'Fixture ownership marker has an invalid ownershipId.'
    }
    if ($State.marker -ne $script:OwnershipMarker) { throw 'Fixture ownership marker type does not match.' }
    if (-not [string]::Equals((Get-CanonicalPath $State.allowedRoot), $Paths.AllowedRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Fixture ownership marker approved root does not match.'
    }
    if (-not [string]::Equals((Get-CanonicalPath $State.runRoot), $Paths.RunRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Fixture ownership marker run root does not match.'
    }
    $ExpectedInstall = Get-CanonicalPath (Join-Path $Paths.RunRoot 'distro')
    if (-not [string]::Equals((Get-CanonicalPath $State.installDir), $ExpectedInstall, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Fixture ownership marker installation path does not match.'
    }
    if ($State.status -ne 'ready' -and -not ($AllowImporting -and $State.status -eq 'importing')) {
        throw 'Fixture ownership marker is not ready.'
    }
    $State
}

function Get-WslDistributionRecord {
    param([Parameter(Mandatory)][string]$DistroName)
    $Root = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss'
    foreach ($Key in Get-ChildItem -LiteralPath $Root -ErrorAction SilentlyContinue) {
        $Value = Get-ItemProperty -LiteralPath $Key.PSPath
        if ([string]::Equals([string]$Value.DistributionName, $DistroName, [StringComparison]::Ordinal)) {
            return [pscustomobject]@{
                Name = [string]$Value.DistributionName
                InstallDir = Get-CanonicalPath ([Environment]::ExpandEnvironmentVariables([string]$Value.BasePath))
                RegistryId = [string]$Key.PSChildName
            }
        }
    }
    return $null
}

function Assert-OwnedDistribution {
    param([Parameter(Mandatory)]$State, [Parameter(Mandatory)]$Distribution)
    if (-not [string]::Equals([string]$Distribution.Name, [string]$State.distroName, [StringComparison]::Ordinal)) {
        throw 'WSL distribution name does not match the ownership record.'
    }
    if (-not [string]::Equals((Get-CanonicalPath $Distribution.InstallDir), (Get-CanonicalPath $State.installDir), [StringComparison]::OrdinalIgnoreCase)) {
        throw 'WSL distribution installation path does not match the owned fixture.'
    }
    if (-not $State.PSObject.Properties['distroRegistryId'] -or
        -not [string]::Equals([string]$Distribution.RegistryId, [string]$State.distroRegistryId, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'WSL distribution registry identity does not match the owned fixture.'
    }
}

function Invoke-OwnedFixtureCleanup {
    param(
        [Parameter(Mandatory)]$State,
        $Distribution,
        [Parameter(Mandatory)][scriptblock]$Terminate,
        [Parameter(Mandatory)][scriptblock]$Unregister,
        [Parameter(Mandatory)][scriptblock]$RemoveDirectory
    )
    if ($null -ne $Distribution) {
        Assert-OwnedDistribution -State $State -Distribution $Distribution
        & $Terminate $State.distroName
        & $Unregister $State.distroName
    }
    & $RemoveDirectory $State.runRoot
}

Export-ModuleMember -Function Assert-FixtureRunRoot, Get-FixtureOwnershipState, Get-WslDistributionRecord, Assert-OwnedDistribution, Invoke-OwnedFixtureCleanup
