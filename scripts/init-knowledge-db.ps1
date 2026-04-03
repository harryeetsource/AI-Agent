param(
    [string]$DatabasePath,
    [string]$SchemaPath
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot

if (-not $DatabasePath) {
    $DatabasePath = Join-Path $RepoRoot "data\knowledge.db"
}
if (-not $SchemaPath) {
    $SchemaPath = Join-Path $RepoRoot "data\schema.sql"
}

if (-not (Test-Path $SchemaPath)) {
    throw "Schema file not found: $SchemaPath"
}

$DatabaseDir = Split-Path -Parent $DatabasePath
if ($DatabaseDir -and -not (Test-Path $DatabaseDir)) {
    New-Item -ItemType Directory -Path $DatabaseDir -Force | Out-Null
}

$sqlite = Get-Command sqlite3 -ErrorAction SilentlyContinue
if (-not $sqlite) {
    throw "sqlite3 was not found in PATH. Install sqlite3 and rerun this script."
}

& sqlite3 $DatabasePath ".read $SchemaPath"
Write-Host "Initialized knowledge database at $DatabasePath"
