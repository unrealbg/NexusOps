[CmdletBinding()]
param([Parameter(Mandatory)][string]$RunRoot)
$ErrorActionPreference = "Stop"
function ConvertTo-WslPath([string]$Path) {
    $Full = [IO.Path]::GetFullPath($Path)
    if ($Full -notmatch '^([A-Za-z]):\\(.*)$') { throw "Unsupported Windows path: $Full" }
    $Drive = $Matches[1].ToLowerInvariant()
    $Rest = $Matches[2].Replace('\', '/')
    return "/mnt/$Drive/$Rest"
}
$RunRoot = (Resolve-Path $RunRoot).Path
$State = Get-Content -Raw (Join-Path $RunRoot "fixture-owner.json") | ConvertFrom-Json
if ($State.marker -ne "nexusops-goal-02a-disposable-openssh" -or $State.runRoot -ne $RunRoot) {
    throw "Fixture ownership marker does not match"
}
$RunRootLinux = ConvertTo-WslPath $RunRoot
Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $RunRoot "sshd.log")
$Arguments = @("-d",$State.distroName,"-u","root","--","/usr/sbin/sshd","-D","-e","-f","/opt/nexusops-fixture/sshd_config","-E","$RunRootLinux/sshd.log")
$Process = Start-Process -FilePath wsl.exe -ArgumentList $Arguments -WindowStyle Hidden -PassThru
$PortLine = Get-Content (Join-Path $RunRoot "fixture-metadata.env") | Where-Object { $_ -like "SSH_PORT=*" }
$Port = [int]($PortLine -replace '^SSH_PORT=', '')
$Deadline = (Get-Date).AddSeconds(20)
do {
    Start-Sleep -Milliseconds 200
    if ($Process.HasExited) { throw "OpenSSH fixture exited during startup" }
    $Ready = Test-NetConnection -ComputerName 127.0.0.1 -Port $Port -InformationLevel Quiet -WarningAction SilentlyContinue
} until ($Ready -or (Get-Date) -gt $Deadline)
if (-not $Ready) { throw "OpenSSH fixture did not listen on 127.0.0.1:$Port" }
[pscustomobject]@{ ProcessId=$Process.Id; Port=$Port; DistroName=$State.distroName }
