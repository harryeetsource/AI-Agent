param(
    [string]$ModelPath = ".\data\models\qwen2.5-coder-1.5b-instruct-q4_k_m.gguf",
    [string]$ServerPath = ".\runners\llama\llama-server.exe",
    [string]$Host = "127.0.0.1",
    [int]$Port = 8081,
    [int]$Context = 8192,
    [int]$Threads = 0
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $ServerPath)) {
    throw "llama server binary not found: $ServerPath"
}

if (-not (Test-Path $ModelPath)) {
    throw "GGUF model not found: $ModelPath"
}

$arguments = @(
    "-m", $ModelPath,
    "--host", $Host,
    "--port", $Port,
    "-c", $Context
)

if ($Threads -gt 0) {
    $arguments += @("-t", $Threads)
}

Write-Host "Starting llama.cpp server..."
Write-Host "Binary: $ServerPath"
Write-Host "Model : $ModelPath"
Write-Host "URL   : http://${Host}:$Port"

& $ServerPath @arguments
