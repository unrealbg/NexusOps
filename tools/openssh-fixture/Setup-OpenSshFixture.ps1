[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AllowedRoot,
    [string]$DistroName,
    [string]$RunRoot
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'FixtureSafety.psm1') -Force

function ConvertTo-WslPath([string]$Path) {
    $Full = [IO.Path]::GetFullPath($Path)
    if ($Full -notmatch '^([A-Za-z]):\\(.*)$') { throw "Unsupported Windows path: $Full" }
    "/mnt/$($Matches[1].ToLowerInvariant())/$($Matches[2].Replace('\', '/'))"
}

if (-not $DistroName) { $DistroName = "NexusOps-OpenSSH-$([Guid]::NewGuid().ToString('N'))" }
if (-not $RunRoot) { $RunRoot = Join-Path $AllowedRoot $DistroName }
$Paths = Assert-FixtureRunRoot -AllowedRoot $AllowedRoot -RunRoot $RunRoot
if (Test-Path -LiteralPath $Paths.RunRoot) { throw "RunRoot already exists: $($Paths.RunRoot)" }
if (Get-WslDistributionRecord -DistroName $DistroName) { throw "WSL distribution already exists: $DistroName" }
New-Item -ItemType Directory -Path $Paths.RunRoot | Out-Null

$RootfsName = 'alpine-minirootfs-3.24.0-x86_64.tar.gz'
$BaseUrl = 'https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/x86_64'
$Rootfs = Join-Path $Paths.RunRoot $RootfsName
$ChecksumFile = "$Rootfs.sha256"
Invoke-WebRequest "$BaseUrl/$RootfsName" -OutFile $Rootfs
Invoke-WebRequest "$BaseUrl/$RootfsName.sha256" -OutFile $ChecksumFile
$Expected = ((Get-Content -LiteralPath $ChecksumFile -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
$Actual = (Get-FileHash -LiteralPath $Rootfs -Algorithm SHA256).Hash.ToLowerInvariant()
if ($Actual -ne $Expected) { throw 'Alpine rootfs SHA-256 mismatch.' }

$InstallDir = Join-Path $Paths.RunRoot 'distro'
$State = [ordered]@{
    marker = 'nexusops-openssh-fixture-v2'
    ownershipId = [Guid]::NewGuid().ToString('D')
    allowedRoot = $Paths.AllowedRoot
    runRoot = $Paths.RunRoot
    installDir = $InstallDir
    distroName = $DistroName
    distroRegistryId = $null
    rootfsSha256 = $Actual
    status = 'importing'
    createdAt = (Get-Date).ToUniversalTime().ToString('o')
}
$MarkerPath = Join-Path $Paths.RunRoot 'fixture-owner.json'
$State | ConvertTo-Json | Set-Content -LiteralPath $MarkerPath -Encoding utf8

& wsl.exe --import $DistroName $InstallDir $Rootfs --version 2 | Out-Host
if ($LASTEXITCODE -ne 0) { throw "wsl --import failed with exit code $LASTEXITCODE" }
$Distribution = Get-WslDistributionRecord -DistroName $DistroName
if (-not $Distribution) { throw 'Imported WSL distribution was not found in the registry.' }
$State.distroRegistryId = $Distribution.RegistryId
$State.status = 'ready'
$State | ConvertTo-Json | Set-Content -LiteralPath $MarkerPath -Encoding utf8
$ReadyState = Get-FixtureOwnershipState -AllowedRoot $Paths.AllowedRoot -RunRoot $Paths.RunRoot
Assert-OwnedDistribution -State $ReadyState -Distribution $Distribution

$PortListener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$PortListener.Start()
$Port = ([Net.IPEndPoint]$PortListener.LocalEndpoint).Port
$PortListener.Stop()
$ConfigureScript = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'configure.sh')).Path
& wsl.exe -d $DistroName -u root -- sh (ConvertTo-WslPath $ConfigureScript) (ConvertTo-WslPath $Paths.RunRoot) $Port | Out-Host
if ($LASTEXITCODE -ne 0) { throw "OpenSSH fixture configuration failed with exit code $LASTEXITCODE" }

[pscustomobject]@{
    DistroName = $DistroName
    RunRoot = $Paths.RunRoot
    AllowedRoot = $Paths.AllowedRoot
    Port = $Port
    RootfsSha256 = $Actual
}
