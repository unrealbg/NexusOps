[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AllowedRoot,
    [Parameter(Mandatory)][string]$RunRoot
)
$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot 'FixtureSafety.psm1') -Force
function ConvertFrom-WslPath([string]$Path) {
    if ($Path -notmatch '^/mnt/([a-zA-Z])/(.*)$') { throw "Unsupported WSL path: $Path" }
    $Drive = $Matches[1].ToUpperInvariant()
    $Rest = $Matches[2].Replace('/', '\')
    return "$Drive`:\$Rest"
}
$State = Get-FixtureOwnershipState -AllowedRoot $AllowedRoot -RunRoot $RunRoot
$RunRoot = $State.runRoot
$Distribution = Get-WslDistributionRecord -DistroName $State.distroName
if (-not $Distribution) { throw 'Owned WSL distribution is not registered.' }
Assert-OwnedDistribution -State $State -Distribution $Distribution
$EnvironmentLinux = "/mnt/$(([IO.Path]::GetPathRoot($RunRoot)[0]).ToString().ToLowerInvariant())/$($RunRoot.Substring(3).Replace('\','/'))/fixture-secrets.env"
$Lines = & wsl.exe -d $State.distroName -u root -- cat $EnvironmentLinux
foreach ($Line in $Lines) {
    if ($Line -match '^([A-Z0-9_]+)=(.*)$') {
        $Name = $Matches[1]
        $Value = $Matches[2]
        if ($Name -like '*_KEY' -or $Name -like '*_DB' -or $Name -like '*_LOG') {
            $Value = ConvertFrom-WslPath $Value
        }
        [Environment]::SetEnvironmentVariable($Name, $Value, 'Process')
    }
}
[Environment]::SetEnvironmentVariable('NEXUS_OPENSSH_DISTRO', $State.distroName, 'Process')
