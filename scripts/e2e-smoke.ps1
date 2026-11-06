# E2E smoke test prerequisites (Windows). Read-only: starts nothing, makes
# no LLM calls, never prints secrets.
#
#   powershell -ExecutionPolicy Bypass -File scripts/e2e-smoke.ps1
#   powershell -ExecutionPolicy Bypass -File scripts/e2e-smoke.ps1 -Image my-image:tag
#
# For a per-agent check (skill.md, allowed tools, the model's key), use:
#   nandi verify --agent echo-agent
# See docs/developer/e2e-checklist.md.

param(
    [string]$Image = 'ghcr.io/vaultdbai/nandi-runtime:sandbox-6'
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# Must match db.rs SCHEMA_VERSION and services/mod.rs APP_IDENTIFIER
$SchemaVersion = 2
$AppIdentifier = 'com.nandi.app'
# llm_keys.rs keyring user for DeepSeek
$DeepSeekKeyringUser = 'nandi.llm.deepseek'

$script:Failures = 0
function Pass([string]$Message) { Write-Output "$([char]0x2705) $Message" }
function Fail([string]$Message) {
    Write-Output "$([char]0x274C) $Message"
    $script:Failures++
}

Write-Output "Nandi e2e smoke prerequisites`n"

# 1. Docker is running
$dockerUp = $false
if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    Fail 'docker CLI not found on PATH. Install Docker Desktop'
} else {
    docker info --format '{{.ServerVersion}}' *> $null
    if ($LASTEXITCODE -eq 0) {
        Pass 'Docker is running'
        $dockerUp = $true
    } else {
        Fail 'Docker is not running. Start Docker Desktop'
    }
}

# 2. Runtime image exists
if ($dockerUp) {
    docker image inspect $Image *> $null
    if ($LASTEXITCODE -eq 0) {
        Pass "Image $Image exists"
    } else {
        Fail "Image $Image not found. Build it with: docker build -t $Image runtimes/nandi-runtime"
    }
} else {
    Fail "Image $Image not checked (Docker unavailable)"
}

# 3. <home>/local/state.db exists with this build's schema. The home is NANDI_HOME
# when set, else ~/.nandi. user_version is bytes 60-63
# of the SQLite header (big-endian), so no sqlite3 CLI is needed. The DB
# runs in WAL mode: a migration not yet checkpointed into the main file can
# read low; `nandi verify` reads it through SQLite.
$nandiHome = if ($env:NANDI_HOME) { $env:NANDI_HOME } else { Join-Path $HOME '.nandi' }
$dbPath = Join-Path $nandiHome 'local\state.db'
if (-not (Test-Path $dbPath)) {
    Fail "$dbPath not found. Launch Nandi once to create it"
} else {
    $stream = [System.IO.File]::Open($dbPath, 'Open', 'Read', 'ReadWrite')
    try {
        $header = New-Object byte[] 64
        $read = $stream.Read($header, 0, 64)
    } finally {
        $stream.Dispose()
    }
    $magic = [System.Text.Encoding]::ASCII.GetString($header, 0, 15)
    if ($read -lt 64 -or $magic -ne 'SQLite format 3') {
        Fail "$dbPath is not a SQLite database"
    } else {
        $version = ([int]$header[60] -shl 24) -bor ([int]$header[61] -shl 16) -bor ([int]$header[62] -shl 8) -bor [int]$header[63]
        if ($version -eq $SchemaVersion) {
            Pass "$dbPath has schema v$version"
        } else {
            Fail "$dbPath has schema v$version, but this build uses v$SchemaVersion. Quit Nandi, delete state.db with its -wal and -shm files, and launch Nandi again"
        }
    }
}

# 4. DeepSeek key stored. The keyring crate names Windows credentials
# '<user>.<service>'; cmdkey lists targets without revealing secrets.
$target = "$DeepSeekKeyringUser.$AppIdentifier"
$listing = cmdkey "/list:$target" 2>$null | Out-String
if ($listing -match [regex]::Escape($target)) {
    Pass 'DeepSeek API key is stored in Windows Credential Manager'
} else {
    Fail 'DeepSeek API key not found. Add it in Nandi: Settings > Models'
}

Write-Output ''
if ($script:Failures -eq 0) {
    Write-Output 'All prerequisites met.'
    exit 0
}
Write-Output "$script:Failures prerequisite(s) missing."
exit 1
