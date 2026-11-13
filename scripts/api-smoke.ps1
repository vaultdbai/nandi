# API smoke test: starts Nandi headless on a throwaway home and calls every
# parameterless GET route and command (`rpc:` rows) of the HTTP API (docs/testing, RL-26). Your real
# ~/.nandi is never used. The OS keyring is shared, so saved provider keys
# are visible to the test instance (read only).
#
#   pnpm smoke:api                       # uses the debug binary (build it first)
#   pnpm smoke:api -- -Binary path\to\nandi.exe -Port 17999
#
# Build first: pnpm run build; cd src-tauri; cargo build --bin nandi

param(
    [string]$Binary = 'src-tauri\target\debug\nandi.exe',
    [string]$TestHome = (Join-Path $env:TEMP 'nandi-smoke-home'),
    [int]$Port = 17999
)

$ErrorActionPreference = 'Stop'

$Routes = @(
    'agents', 'agents/overviews', 'approvals/pending', 'ask/history', 'aws/cli',
    'aws/profiles', 'bedrock/gateway', 'bedrock/models', 'capabilities', 'companion/snapshot',
    'connectors', 'default-model', 'delegation/history', 'delegation/providers',
    'gateway/callers', 'gateway/config', 'gateway/holds', 'gateway/manifest', 'gateway/status',
    'harnesses', 'llm-keys', 'marketplace/agents', 'marketplace/installed',
    'marketplace/packs', 'marketplace/updates', 'ollama', 'pilots', 'preferences',
    'relay/agents', 'relay/status', 'repo/git/status', 'repo/status', 'studio/push-targets',
    'teams', 'triggers', 'workflows',
    # Commands without arguments: `POST /api/rpc/{name}` with `{}`
    'rpc:get_auto_approve_rules', 'rpc:get_chief_of_staff_digest', 'rpc:get_egress_status',
    'rpc:get_identity_status', 'rpc:get_know_me', 'rpc:get_my_notes',
    'rpc:get_network_snapshot', 'rpc:get_pending_clarifications', 'rpc:get_remote_status',
    'rpc:get_repo_llm_config', 'rpc:get_tool_usage_stats', 'rpc:get_workspace_status',
    'rpc:help_topics', 'rpc:list_agents_shared_by_me', 'rpc:list_agents_shared_with_me',
    'rpc:list_capability_providers', 'rpc:list_capability_registries',
    'rpc:list_connector_grants', 'rpc:list_decision_months', 'rpc:list_declared_capabilities',
    'rpc:list_dismissed_work', 'rpc:list_harness_sessions', 'rpc:list_note_suggestions',
    'rpc:list_pending_manual_steps', 'rpc:list_recent_changes', 'rpc:list_tasks',
    'rpc:list_tools', 'rpc:list_trust_policies', 'rpc:list_trust_requests', 'rpc:list_work',
    'rpc:mascot_get'
)

if (-not (Test-Path $Binary)) {
    Write-Output "Binary not found: $Binary. Build it first (see the top of this script)."
    exit 2
}
$realHome = Join-Path $env:USERPROFILE '.nandi'
if ((Resolve-Path -LiteralPath $TestHome -ErrorAction SilentlyContinue) -and
    ((Resolve-Path -LiteralPath $TestHome).Path -eq (Resolve-Path -LiteralPath $realHome -ErrorAction SilentlyContinue).Path)) {
    Write-Output "Refusing to use your real Nandi home ($realHome) as the test home."
    exit 2
}

if (Test-Path $TestHome) { Remove-Item -Recurse -Force $TestHome }
New-Item -ItemType Directory -Force $TestHome | Out-Null

$env:NANDI_HOME = $TestHome
$server = Start-Process -FilePath $Binary -ArgumentList '--headless', '--port', $Port `
    -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $TestHome 'smoke-server.out') `
    -RedirectStandardError (Join-Path $TestHome 'smoke-server.err')

try {
    $tokenFile = Join-Path $TestHome 'web_token'
    $ready = $false
    for ($i = 0; $i -lt 90 -and -not $ready; $i++) {
        Start-Sleep -Seconds 1
        if (Test-Path $tokenFile) {
            try { Invoke-WebRequest "http://127.0.0.1:$Port/" -UseBasicParsing -TimeoutSec 2 | Out-Null; $ready = $true } catch { $ready = $_.Exception.Response -ne $null }
        }
    }
    if (-not $ready) { Write-Output 'The server did not start. See smoke-server.err in the test home.'; exit 1 }

    $headers = @{ Authorization = "Bearer $((Get-Content $tokenFile -Raw).Trim())" }
    $failed = 0
    foreach ($route in $Routes) {
        try {
            $response = if ($route.StartsWith('rpc:')) {
                Invoke-WebRequest "http://127.0.0.1:$Port/api/rpc/$($route.Substring(4))" -Method Post -Body '{}' `
                    -ContentType 'application/json' -Headers $headers -UseBasicParsing -TimeoutSec 60
            } else {
                Invoke-WebRequest "http://127.0.0.1:$Port/api/$route" -Headers $headers -UseBasicParsing -TimeoutSec 60
            }
            Write-Output ("{0}  {1}" -f $response.StatusCode, $route)
        } catch {
            $code = if ($_.Exception.Response) { [int]$_.Exception.Response.StatusCode } else { 0 }
            Write-Output ("{0}  {1}  <-- FAIL" -f $code, $route)
            $failed++
        }
    }
    try {
        Invoke-WebRequest "http://127.0.0.1:$Port/api/agents" -UseBasicParsing -TimeoutSec 5 | Out-Null
        Write-Output 'Without a token the API answered: FAIL (expected 401)'
        $failed++
    } catch {
        Write-Output ("{0}  (no token; 401 expected)" -f [int]$_.Exception.Response.StatusCode)
    }

    Write-Output ''
    Write-Output ("{0} of {1} routes answered 200." -f ($Routes.Count - $failed), $Routes.Count)
    if ($failed -gt 0) { exit 1 }
} finally {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
}
