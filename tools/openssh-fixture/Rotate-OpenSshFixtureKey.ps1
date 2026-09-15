[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AllowedRoot,
    [Parameter(Mandatory)][string]$RunRoot
)
$ErrorActionPreference = "Stop"
& (Join-Path $PSScriptRoot "Set-OpenSshFixtureKey.ps1") -AllowedRoot $AllowedRoot -RunRoot $RunRoot -Key B
