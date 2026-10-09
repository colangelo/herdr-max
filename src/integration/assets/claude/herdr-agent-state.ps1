# installed by herdr
# managed by herdr; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# HERDR_INTEGRATION_ID=claude
# HERDR_INTEGRATION_VERSION=11

param([string]$Action = "")

if ($Action -ne "session") { exit 0 }
if ($env:HERDR_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:HERDR_PANE_ID)) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    exit 0
}

$propertyNames = @($payload.PSObject.Properties.Name)
if ((Test-Path Env:CURSOR_VERSION) -or $propertyNames -ccontains "cursor_version") { exit 0 }
if (-not [string]::IsNullOrWhiteSpace($payload.agent_id)) { exit 0 }
# A `claude -p` started from a session's Bash tool inherits HERDR_PANE_ID and
# runs these hooks too (fork issue 143); it is not the pane's agent. Claude sets
# this entrypoint for a print-mode run.
if ($env:CLAUDE_CODE_ENTRYPOINT -eq "sdk-cli") { exit 0 }
# Upstream c5051933: only Claude's own, exactly-spelled events count (the fork
# installs SessionStart, UserPromptSubmit and Stop).
if (-not ($propertyNames -ccontains "hook_event_name") -or $payload.hook_event_name -isnot [string] -or @("SessionStart", "UserPromptSubmit", "Stop") -cnotcontains $payload.hook_event_name) { exit 0 }

$sessionId = $payload.session_id
if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

# The command that resumes this session as it is now: its permission mode,
# model and effort (fork issue 123). Values come from the hook input when it
# has them, then from the end of the transcript. Unlike the POSIX hook this one
# does not read Claude's launch flags, so bypass is kept only when the session
# was seen in it.
$resumeModes = @("acceptEdits", "auto", "bypassPermissions", "default", "manual", "dontAsk", "plan")
$resumeEfforts = @("low", "medium", "high", "xhigh", "max")
$plainValue = '^[A-Za-z0-9._:/\[\]-]{1,200}$'
function Get-PlainValue($value) {
    if ($value -is [string] -and $value -cmatch $plainValue) { return $value }
    return $null
}
$transcriptMode = $null
$transcriptModel = $null
$transcriptEffort = $null
$sawBypass = $false
if ($payload.transcript_path -is [string] -and (Test-Path -LiteralPath $payload.transcript_path)) {
    try {
        $stream = [System.IO.File]::Open($payload.transcript_path, 'Open', 'Read', 'ReadWrite')
        try {
            $start = [Math]::Max(0, $stream.Length - 524288)
            $null = $stream.Seek($start, 'Begin')
            $reader = New-Object System.IO.StreamReader($stream)
            $lines = $reader.ReadToEnd() -split "`n"
        } finally {
            $stream.Dispose()
        }
        if ($start -gt 0) { $lines = $lines | Select-Object -Skip 1 }
        foreach ($raw in $lines) {
            if ([string]::IsNullOrWhiteSpace($raw)) { continue }
            try { $entry = $raw | ConvertFrom-Json } catch { continue }
            if ($entry.isSidechain) { continue }
            if ($entry.type -eq "user" -and $resumeModes -ccontains $entry.permissionMode) {
                $transcriptMode = $entry.permissionMode
                if ($transcriptMode -eq "bypassPermissions") { $sawBypass = $true }
            } elseif ($entry.type -eq "assistant") {
                $model = Get-PlainValue $entry.message.model
                if ($model) { $transcriptModel = $model }
                if ($resumeEfforts -ccontains $entry.effort) { $transcriptEffort = $entry.effort }
            }
        }
    } catch {
    }
}
$mode = @($payload.permission_mode, $transcriptMode) | Where-Object { $resumeModes -ccontains $_ } | Select-Object -First 1
$model = Get-PlainValue $payload.model
if (-not $model) { $model = $transcriptModel }
$resumeArgv = @()
if (Get-PlainValue $sessionId) {
    $resumeArgv = @("claude", "--resume", "$sessionId")
    if ($model) { $resumeArgv += @("--model", $model) }
    if ($transcriptEffort) { $resumeArgv += @("--effort", $transcriptEffort) }
    if ($mode -eq "bypassPermissions" -or $sawBypass) { $resumeArgv += "--allow-dangerously-skip-permissions" }
    if ($mode) { $resumeArgv += @("--permission-mode", $mode) }
}

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$herdr = if ([string]::IsNullOrWhiteSpace($env:HERDR_BIN_PATH)) { "herdr" } else { $env:HERDR_BIN_PATH }
try {
    $args = @(
        "pane",
        "report-agent-session",
        $env:HERDR_PANE_ID,
        "--source",
        "herdr:claude",
        "--agent",
        "claude",
        "--seq",
        "$seq",
        "--agent-session-id",
        "$sessionId"
    )
    if ($payload.transcript_path -is [string] -and -not [string]::IsNullOrWhiteSpace($payload.transcript_path)) {
        $args += @("--agent-session-path", "$($payload.transcript_path)")
    }
    if ($payload.hook_event_name -eq "SessionStart" -and $payload.source -is [string] -and -not [string]::IsNullOrWhiteSpace($payload.source)) {
        $args += @("--session-start-source", "$($payload.source)")
    }
    if ($resumeArgv.Count -gt 0) {
        $args += "--"
        $args += $resumeArgv
    }
    & $herdr @args 2>$null | Out-Null
} catch {
}
