## 1. Server

- [x] 1.1 `App::pane_agent_resume_plan`: `restore_plan_preview` plus the live Claude footer
- [x] 1.2 `App::respawn_pane_agent`: stop the runtime, set the pending plan, start it as a restore does
- [x] 1.3 `Method::PaneRespawnAgent` (`pane.respawn_agent`), appended; UI-changing, not client-shell admitted
- [x] 1.4 Handler: `no_agent_session` before the live-work check; same `respawned` result

## 2. CLI and docs

- [x] 2.1 `herdr pane respawn --resume` sends `pane.respawn_agent`
- [x] 2.2 `cli-reference.mdx`, `socket-api.mdx`, regenerated `herdr-api.schema.json`

## 3. Verification

- [x] 3.1 Unit tests: resume command typed and records kept; no-session refusal; CLI mapping
- [x] 3.2 Live throwaway on the beta: an agent pane respawned with `--resume` comes back under the new server
