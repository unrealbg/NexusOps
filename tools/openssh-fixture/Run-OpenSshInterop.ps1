[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AllowedRoot,
    [Parameter(Mandatory)][string]$RunRoot
)
$ErrorActionPreference = "Stop"
$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
. (Join-Path $PSScriptRoot "Import-FixtureEnvironment.ps1") -AllowedRoot $AllowedRoot -RunRoot $RunRoot
Push-Location $ProjectRoot
try {
    cargo test -p nexus-ssh --test openssh_interop openssh_phase_a --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH phase A failed" }
    $SystemctlPath = & wsl.exe -d $env:NEXUS_OPENSSH_DISTRO -u $env:NEXUS_OPENSSH_USER -- sh -lc 'command -v systemctl 2>/dev/null'
    $SystemctlProbeExit = $LASTEXITCODE
    if ($SystemctlProbeExit -eq 0 -or -not [string]::IsNullOrWhiteSpace(($SystemctlPath -join ''))) {
        throw 'OpenSSH fixture systemctl-absence gate failed before Goal 05B transport dispatch'
    }
    [Environment]::SetEnvironmentVariable('NEXUS_OPENSSH_SYSTEMCTL_ABSENT', '1', 'Process')
    cargo test -p nexus-ssh --test openssh_interop openssh_systemd_reset_failed_non_mutating_completion --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH Goal 05B non-mutating completion interoperability failed" }
    cargo test -p nexus-ssh --test openssh_interop openssh_systemd_try_restart_non_mutating_completion --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH Goal 05D non-mutating completion interoperability failed" }
    cargo test -p nexus-ssh --test openssh_interop openssh_systemd_reload_non_mutating_completion --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH Goal 05E non-mutating completion interoperability failed" }
    cargo test -p nexus-ssh --test openssh_interop openssh_systemd_start_non_mutating_completion --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH Goal 05F non-mutating completion interoperability failed" }
    cargo test -p nexus-ssh --test openssh_monitoring openssh_monitoring_interoperability --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH monitoring interoperability failed" }
    cargo test -p nexus-ssh --test openssh_terminal openssh_terminal_pty_interoperability --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH terminal interoperability failed" }
    cargo test -p nexus-ssh --test openssh_sftp openssh_sftp_streaming_interoperability --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH SFTP interoperability failed" }
    & (Join-Path $PSScriptRoot "Rotate-OpenSshFixtureKey.ps1") -AllowedRoot $AllowedRoot -RunRoot $RunRoot | Out-Host
    cargo test -p nexus-ssh --test openssh_interop openssh_phase_b_changed_key --locked -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH phase B failed" }
} finally {
    [Environment]::SetEnvironmentVariable('NEXUS_OPENSSH_SYSTEMCTL_ABSENT', $null, 'Process')
    Pop-Location
}
