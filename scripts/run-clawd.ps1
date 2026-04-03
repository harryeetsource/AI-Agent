param(
    [string]$RunnerBaseUrl = "http://127.0.0.1:8081",
    [string]$RunnerMessagesPath = "/v1/messages",
    [string]$BindHost = "127.0.0.1",
    [int]$Port = 8080
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot

$env:CLAW_RUNNER_BASE_URL = $RunnerBaseUrl
$env:CLAW_RUNNER_MESSAGES_PATH = $RunnerMessagesPath
$env:CLAWD_HOST = $BindHost
$env:CLAWD_PORT = "$Port"
$env:CLAW_LOCAL_BASE_URL = "http://${BindHost}:$Port"

$ClawdPath = Join-Path $RepoRoot "target\release\clawd.exe"

if (-not (Test-Path $ClawdPath)) {
    throw "clawd binary not found: $ClawdPath"
}

Write-Host "Starting clawd..."
Write-Host "Binary: $ClawdPath"
Write-Host "URL   : http://${BindHost}:$Port"

& $ClawdPath
