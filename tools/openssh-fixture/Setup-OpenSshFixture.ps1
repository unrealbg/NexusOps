[CmdletBinding()]
param(
    [string]$DistroName = "NexusOps-Goal02A-20260914",
    [string]$RunRoot
)
$ErrorActionPreference = "Stop"
function ConvertTo-WslPath([string]$Path) {
    $Full = [IO.Path]::GetFullPath($Path)
    if ($Full -notmatch '^([A-Za-z]):\\(.*)$') { throw "Unsupported Windows path: $Full" }
    $Drive = $Matches[1].ToLowerInvariant()
    $Rest = $Matches[2].Replace('\', '/')
    return "/mnt/$Drive/$Rest"
}
$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$TaskRoot = Split-Path (Split-Path $ProjectRoot -Parent) -Parent
if (-not $RunRoot) { $RunRoot = Join-Path $TaskRoot "work\goal-02a-openssh" }
$RunRoot = [IO.Path]::GetFullPath($RunRoot)
$AllowedRoot = [IO.Path]::GetFullPath((Join-Path $TaskRoot "work"))
if (-not $RunRoot.StartsWith($AllowedRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "RunRoot must be inside $AllowedRoot"
}
if (Test-Path $RunRoot) { throw "RunRoot already exists: $RunRoot" }
New-Item -ItemType Directory -Path $RunRoot | Out-Null

$RootfsName = "alpine-minirootfs-3.24.0-x86_64.tar.gz"
$BaseUrl = "https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/x86_64"
$Rootfs = Join-Path $RunRoot $RootfsName
$ChecksumFile = "$Rootfs.sha256"
Invoke-WebRequest "$BaseUrl/$RootfsName" -OutFile $Rootfs
Invoke-WebRequest "$BaseUrl/$RootfsName.sha256" -OutFile $ChecksumFile
$Expected = ((Get-Content -Raw $ChecksumFile).Trim() -split '\s+')[0].ToLowerInvariant()
$Actual = (Get-FileHash -Algorithm SHA256 $Rootfs).Hash.ToLowerInvariant()
if ($Actual -ne $Expected) { throw "Alpine rootfs SHA-256 mismatch" }

$Existing = @(& wsl.exe --list --quiet | ForEach-Object { $_.Trim("`0 ") } | Where-Object { $_ })
if ($Existing -contains $DistroName) { throw "WSL distribution already exists: $DistroName" }
$InstallDir = Join-Path $RunRoot "distro"
$State = [ordered]@{
    marker = "nexusops-goal-02a-disposable-openssh"
    distroName = $DistroName
    runRoot = $RunRoot
    installDir = $InstallDir
    rootfsSha256 = $Actual
    createdAt = (Get-Date).ToUniversalTime().ToString("o")
}
$State | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $RunRoot "fixture-owner.json")

& wsl.exe --import $DistroName $InstallDir $Rootfs --version 2
if ($LASTEXITCODE -ne 0) { throw "wsl --import failed with exit code $LASTEXITCODE" }

$PortListener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$PortListener.Start()
$Port = ([Net.IPEndPoint]$PortListener.LocalEndpoint).Port
$PortListener.Stop()
$ConfigureScript = (Resolve-Path (Join-Path $PSScriptRoot "configure.sh")).Path
$ConfigureLinux = ConvertTo-WslPath $ConfigureScript
$RunRootLinux = ConvertTo-WslPath $RunRoot
& wsl.exe -d $DistroName -u root -- sh $ConfigureLinux $RunRootLinux $Port
if ($LASTEXITCODE -ne 0) { throw "OpenSSH fixture configuration failed with exit code $LASTEXITCODE" }

[pscustomobject]@{ DistroName=$DistroName; RunRoot=$RunRoot; Port=$Port; RootfsSha256=$Actual }
