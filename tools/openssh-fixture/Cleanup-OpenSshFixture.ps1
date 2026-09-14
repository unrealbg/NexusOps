[CmdletBinding()]
param([Parameter(Mandatory)][string]$RunRoot)
$ErrorActionPreference = "Stop"
$RunRoot = (Resolve-Path $RunRoot).Path
$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$TaskRoot = Split-Path (Split-Path $ProjectRoot -Parent) -Parent
$AllowedRoot = [IO.Path]::GetFullPath((Join-Path $TaskRoot "work"))
if (-not $RunRoot.StartsWith($AllowedRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing cleanup outside $AllowedRoot"
}
$State = Get-Content -Raw (Join-Path $RunRoot "fixture-owner.json") | ConvertFrom-Json
if ($State.marker -ne "nexusops-goal-02a-disposable-openssh" -or $State.runRoot -ne $RunRoot) {
    throw "Fixture ownership marker does not match"
}
$Existing = @(& wsl.exe --list --quiet | ForEach-Object { $_.Trim("`0 ") } | Where-Object { $_ })
if ($Existing -contains $State.distroName) {
    & wsl.exe --terminate $State.distroName 2>$null
    & wsl.exe --unregister $State.distroName
    if ($LASTEXITCODE -ne 0) { throw "Failed to unregister $($State.distroName)" }
}
Remove-Item -LiteralPath $RunRoot -Recurse -Force
