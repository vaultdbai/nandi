# API smoke test: starts Nandi headless on a throwaway home and calls every
# command without arguments over the HTTP API (docs/testing, RL-26). Your real
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

# Commands without arguments, each sent as `POST /api/rpc/{name}` with `{}`
$Commands = @(
    'check_for_updates', 'detect_aws_cli', 'detect_ollama', 'get_auto_approve_rules',
    'get_chief_of_staff_digest', 'get_default_model', 'get_egress_status',
    'get_email_status', 'get_gateway_config', 'get_gateway_manifest', 'get_gateway_status',
    'get_identity_status', 'get_know_me', 'get_litellm_gateway_status',
    'get_llm_key_status', 'get_my_notes', 'get_network_snapshot',
    'get_pending_clarifications', 'get_relay_status', 'get_remote_status',
    'get_repo_llm_config', 'get_studio_conversations', 'get_tool_usage_stats',
    'get_workflows', 'get_workspace_status', 'help_topics', 'list_agents',
    'list_agents_shared_by_me', 'list_agents_shared_with_me', 'list_ask_history',
    'list_available_packs', 'list_aws_profiles', 'list_bedrock_models', 'list_capabilities',
    'list_capability_providers', 'list_capability_registries', 'list_connector_grants',
    'list_connectors', 'list_decision_months', 'list_declared_capabilities',
    'list_dismissed_work', 'list_gateway_callers', 'list_gateway_holds', 'list_groups',
    'list_harness_sessions', 'list_harnesses', 'list_installed', 'list_note_suggestions',
    'list_online_agents', 'list_pending_approvals', 'list_pending_manual_steps',
    'list_pilots', 'list_push_targets', 'list_recent_changes', 'list_request_returns',
    'list_tasks', 'list_teams', 'list_tools', 'list_triggers', 'list_trust_policies',
    'list_trust_requests', 'list_work', 'list_workflow_templates', 'load_preferences',
    'mascot_get', 'repo_git_status', 'repo_status'
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
    foreach ($command in $Commands) {
        try {
            $response = Invoke-WebRequest "http://127.0.0.1:$Port/api/rpc/$command" -Method Post -Body '{}' `
                -ContentType 'application/json' -Headers $headers -UseBasicParsing -TimeoutSec 60
            Write-Output ("{0}  {1}" -f $response.StatusCode, $command)
        } catch {
            $code = if ($_.Exception.Response) { [int]$_.Exception.Response.StatusCode } else { 0 }
            Write-Output ("{0}  {1}  <-- FAIL" -f $code, $command)
            $failed++
        }
    }
    try {
        Invoke-WebRequest "http://127.0.0.1:$Port/api/rpc/list_agents" -Method Post -Body '{}' `
            -ContentType 'application/json' -UseBasicParsing -TimeoutSec 5 | Out-Null
        Write-Output 'Without a token the API answered: FAIL (expected 401)'
        $failed++
    } catch {
        Write-Output ("{0}  (no token; 401 expected)" -f [int]$_.Exception.Response.StatusCode)
    }

    Write-Output ''
    Write-Output ("{0} of {1} commands answered 200." -f ($Commands.Count - $failed), $Commands.Count)
    if ($failed -gt 0) { exit 1 }
} finally {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
}
