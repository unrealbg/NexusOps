[CmdletBinding()]
param([Parameter(Mandatory)][string]$RunRoot)
$ErrorActionPreference = "Stop"
function ConvertFrom-WslPath([string]$Path) {
    if ($Path -notmatch '^/mnt/([a-zA-Z])/(.*)$') { throw "Unsupported WSL path: $Path" }
    $Drive = $Matches[1].ToUpperInvariant()
    $Rest = $Matches[2].Replace('/', '\')
    return "$Drive`:\$Rest"
}
$RunRoot = (Resolve-Path $RunRoot).Path
$State = Get-Content -Raw (Join-Path $RunRoot "fixture-owner.json") | ConvertFrom-Json
if ($State.marker -ne "nexusops-goal-02a-disposable-openssh" -or $State.runRoot -ne $RunRoot) {
    throw "Fixture ownership marker does not match"
}
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
