param(
    [ValidateSet("start-sharer", "collect", "stop")]
    [string]$Action,
    [string]$Repo = "C:\workspace\lan-meeting",
    [string]$ArtifactDir = "artifacts\e2e-screen-probe\latest",
    [int]$DurationMs = 25000,
    [int]$DisplayId = 0,
    [int]$Fps = 30,
    [string]$StdoutName = "windows-sharer.json"
)

$ErrorActionPreference = "Stop"

Set-Location $Repo
$artifactPath = Join-Path $Repo $ArtifactDir
New-Item -ItemType Directory -Force -Path $artifactPath | Out-Null

$stdoutPath = Join-Path $artifactPath "windows-sharer.json"
$stderrPath = Join-Path $artifactPath "windows-sharer.log"
$buildLogPath = Join-Path $artifactPath "windows-build.log"
$buildOutPath = Join-Path $artifactPath "windows-build.out.log"
$buildErrPath = Join-Path $artifactPath "windows-build.err.log"
$pidPath = Join-Path $artifactPath "windows-sharer.pid"

if ($Action -eq "stop") {
    Get-Process e2e_probe -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Get-Process lan-meeting -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Get-Process cargo -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -like "*\.cargo\bin\cargo.exe" -or $_.ProcessName -eq "cargo" } |
        Stop-Process -Force -ErrorAction SilentlyContinue

    [pscustomobject]@{
        action = $Action
        stopped = $true
        artifact_dir = $artifactPath
    } | ConvertTo-Json -Depth 4
    exit 0
}

if ($Action -eq "collect") {
    $target = Join-Path $artifactPath $StdoutName
    if (Test-Path $target) {
        Get-Content -Raw -Encoding utf8 $target
    } else {
        [pscustomobject]@{
            action = $Action
            found = $false
            missing = $target
        } | ConvertTo-Json -Depth 4
    }
    exit 0
}

Get-Process e2e_probe -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process lan-meeting -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

Remove-Item $stdoutPath -ErrorAction SilentlyContinue
Remove-Item $stderrPath -ErrorAction SilentlyContinue
Remove-Item $buildLogPath -ErrorAction SilentlyContinue
Remove-Item $buildOutPath -ErrorAction SilentlyContinue
Remove-Item $buildErrPath -ErrorAction SilentlyContinue
Remove-Item $pidPath -ErrorAction SilentlyContinue

$buildProcess = Start-Process cargo `
    -ArgumentList @("build", "--manifest-path", "src-tauri\Cargo.toml", "--bin", "e2e_probe", "--no-default-features") `
    -RedirectStandardOutput $buildOutPath `
    -RedirectStandardError $buildErrPath `
    -Wait `
    -PassThru `
    -NoNewWindow

Get-Content -Raw -Encoding utf8 $buildOutPath -ErrorAction SilentlyContinue | Set-Content -Path $buildLogPath -Encoding utf8
Get-Content -Raw -Encoding utf8 $buildErrPath -ErrorAction SilentlyContinue | Add-Content -Path $buildLogPath -Encoding utf8

if ($buildProcess.ExitCode -ne 0) {
    [pscustomobject]@{
        action = $Action
        started = $false
        build_failed = $true
        build_log = $buildLogPath
        exit_code = $buildProcess.ExitCode
    } | ConvertTo-Json -Depth 4
    exit $buildProcess.ExitCode
}

$probeExe = Join-Path $Repo "src-tauri\target\debug\e2e_probe.exe"
$command = @(
    "Set-Location '$Repo';",
    "& '$probeExe'",
    "--mode sharer",
    "--duration-ms $DurationMs",
    "--display-id $DisplayId",
    "--fps $Fps",
    "--json",
    "1> '$stdoutPath'",
    "2> '$stderrPath'"
) -join " "

$process = Start-Process powershell `
    -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", $command) `
    -PassThru `
    -WindowStyle Hidden

Set-Content -Path $pidPath -Value $process.Id -Encoding ascii

[pscustomobject]@{
    action = $Action
    pid = $process.Id
    repo = $Repo
    artifact_dir = $artifactPath
    stdout = $stdoutPath
    stderr = $stderrPath
    build_log = $buildLogPath
    duration_ms = $DurationMs
    display_id = $DisplayId
    fps = $Fps
} | ConvertTo-Json -Depth 4
