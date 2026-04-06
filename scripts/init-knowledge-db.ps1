param(
    [string]$DatabasePath = ".\data\knowledge.db",
    [string]$SchemaPath = ".\data\schema.sql"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $SchemaPath)) {
    throw "Schema file not found: $SchemaPath"
}

$sqlite = Get-Command sqlite3 -ErrorAction SilentlyContinue
if (-not $sqlite) {
    throw "sqlite3 was not found in PATH. Install sqlite3 and rerun this script."
}

& sqlite3 $DatabasePath ".read $SchemaPath"
Write-Host "Initialized knowledge database at $DatabasePath"
