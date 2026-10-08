# Tasks: remote-binary-name-discovery

> Backfilled after implementation (`6baf691a`); all items were completed and
> verified before these artifacts were written.

## 1. Name resolution

- [x] 1.1 `remote_binary_names()` — invoked name (from `current_exe`) first, canonical `herdr` fallback, deduped
- [x] 1.2 `sanitized_binary_name()` — plain `[A-Za-z0-9._-]`, non-empty, not `-`-leading; anything else falls back to canonical

## 2. Discovery and install

- [x] 2.1 `remote_binary_on_path_any` takes a name (`command -v <name>`); `remote_binary_candidates` iterates all names
- [x] 2.2 `known_remote_binary_candidate_script` emits every known install path per name
- [x] 2.3 `RemoteHerdr::for_platform_named` — managed install target `~/.local/bin/<name>`; `prepare_remote_herdr` resolves the install name

## 3. Validation

- [x] 3.1 Tests: multi-name script generation, name-following install path, canonical fallback + dedup, metacharacter rejection
- [x] 3.2 `just check`
- [x] 3.3 Live verification: both Macs on the same build, `herdr-beta --remote` attached with no install prompt and no server stop; `~/.local/bin/herdr-beta` absent afterwards and the remote server process not restarted (issue #22 closed)
