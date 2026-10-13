## Why

Upstream 241b99cd ("refactor: share cli and api command contracts", herdrdev/herdr #5212) rebuilds
the CLI on the API schema: every socket method is declared once in a `define_operations!` table
(name, `changes_ui`, `client_shell`), and every CLI subcommand becomes a `Leaf` whose clap
arguments are a `cli_projection!` of the method's params. A plain merge of it into the fork gives
16 conflicting files, and worse, a merge that "resolves" them by taking upstream drops every fork
command that the old hand-written parsers carried. This is step 2 of the 2026-10-13 weekly sync:
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/218 (step 1 was
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/217, landed at `sync-merge/57`).

## What Changes

- Merge upstream 241b99cd onto fork master (base: step 1, `f3fb2099`), taking upstream's
  contract layer wholesale.
- Port each fork-only API method into `define_operations!` with the right `changes_ui` and
  `client_shell` flags. This replaces the fork's hand-kept lists (`CLIENT_SHELL_METHODS`, the
  UI-changing method list), so the #209 guard (every client-shell method is advertised) must hold
  through the table.
- Port each fork-only CLI command and flag onto a `Leaf` + projection in the matching
  `*_bindings.rs`, or keep it as a custom command next to the leaves when it is not one method
  call (multi-step, client-side chunking, local editors).
- Keep every fork wire shape: `close` methods' optional `force`, `pane.clear_scrollback`, and the
  v1 frozen shapes in `advertised_client_shell_method_shapes_stay_at_the_v1_contract`.

## Capabilities

### Modified Capabilities

- `cli-contracts`: the fork's CLI commands and API methods ride upstream's shared contract layer
  with no change in names, flags, output or wire shape.

## Impact

`src/api/schema*.rs`, `src/api/server.rs`, `src/api/mod.rs`, `src/cli/**` (new `*_bindings.rs`,
`projection.rs`), `src/server/client_commands.rs`, `tests/cli/**`, CLI docs. No protocol bump: the
wire format is unchanged. User-facing: none intended; a beta is cut only to carry it.
