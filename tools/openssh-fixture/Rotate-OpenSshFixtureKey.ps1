[CmdletBinding()]
param([Parameter(Mandatory)][string]$RunRoot)
$ErrorActionPreference = "Stop"
& (Join-Path $PSScriptRoot "Set-OpenSshFixtureKey.ps1") -RunRoot $RunRoot -Key B
