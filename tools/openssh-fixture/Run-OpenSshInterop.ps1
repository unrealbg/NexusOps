[CmdletBinding()]
param([Parameter(Mandatory)][string]$RunRoot)
$ErrorActionPreference = "Stop"
$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$TaskRoot = Split-Path (Split-Path $ProjectRoot -Parent) -Parent
. (Join-Path $TaskRoot "work\build-env.ps1")
. (Join-Path $PSScriptRoot "Import-FixtureEnvironment.ps1") -RunRoot $RunRoot
Push-Location $ProjectRoot
try {
    cargo test -p nexus-ssh --test openssh_interop openssh_phase_a --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH phase A failed" }
    cargo test -p nexus-ssh --test openssh_terminal openssh_terminal_pty_interoperability --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH terminal interoperability failed" }
    & (Join-Path $PSScriptRoot "Rotate-OpenSshFixtureKey.ps1") -RunRoot $RunRoot | Out-Host
    cargo test -p nexus-ssh --test openssh_interop openssh_phase_b_changed_key --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH phase B failed" }
} finally {
    Pop-Location
}
