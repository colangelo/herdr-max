param(
    [ValidateSet("lint", "check")]
    [string]$Mode = "check"
)

$ErrorActionPreference = "Stop"

function Invoke-Checked {
    param([string]$Command, [string[]]$Arguments)

    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "command failed with exit code $LASTEXITCODE`: $Command $($Arguments -join ' ')"
    }
}

function Invoke-CargoWithZigCacheRecovery {
    param([string[]]$Arguments)

    & cargo @Arguments
    if ($LASTEXITCODE -eq 0) {
        return
    }

    Write-Warning "cargo compile failed; clearing Zig build caches and retrying once"
    Remove-Item -Recurse -Force .zig-cache -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force vendor/libghostty-vt/.zig-cache -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force vendor/libghostty-vt/zig-out -ErrorAction SilentlyContinue
    Invoke-Checked cargo $Arguments
}

# THROWAWAY PROBE (Herdr Max issue 188): run only the respawn probe tests, live
# output, no fmt/clippy/retry. Never merge this.
& cargo nextest run --locked `
    -E "test(respawn_pane_runtime_falls_back_to_a_shell_without_launch_argv) | test(respawn_probe_settled_shell_shutdown) | test(respawn_does_not_pull_focus_to_the_pane_s_workspace) | test(replaced_runtime_exit_does_not_close_the_respawned_pane) | test(respawn_pane_runtime_clears_agent_runtime_identity) | test(respawn_pane_runtime_shell_target_ignores_the_launch_argv)" `
    --no-fail-fast --no-capture --status-level all --final-status-level all
if ($LASTEXITCODE -ne 0) {
    throw "respawn probe: nextest exited with code $LASTEXITCODE"
}
return

Invoke-Checked cargo @("fmt", "--check")
Invoke-CargoWithZigCacheRecovery @(
    "clippy",
    "--all-targets",
    "--locked",
    "--",
    "-D",
    "warnings"
)

if ($Mode -eq "lint") {
    return
}

Invoke-Checked just @("test")
Invoke-Checked cargo @("build", "--locked")
