## MODIFIED Requirements

### Requirement: Nested lookup stays off the unwrapped path

The nested lookup SHALL NOT add work to panes that are not wrapped, beyond
comparing the foreground process group leader's name against the recognised set.

The lookup SHALL be reached only through the existing foreground-job probe. For a
pane whose last probe found a recognised wrapper, detection SHALL read the
foreground process group of the shell that wrapper hosts once per tick, and a
change of that group, or the shell's disappearance, SHALL trigger a probe and
count as a foreground change.

#### Scenario: No additional sampling is introduced

- **WHEN** the foreground-job probe is skipped for a pane that is not wrapped
- **THEN** no nested lookup is performed for that pane

#### Scenario: A command starting behind a wrapper is probed

- **WHEN** the shell a wrapper hosts starts a command in a pane with no identified agent
- **THEN** the next tick probes the pane without waiting for screen content to change

## ADDED Requirements

### Requirement: agent start accepts a shell behind a recognised wrapper

`agent start` SHALL accept a pane whose foreground job is a recognised PTY
wrapper alone in its group when the shell that wrapper hosts is alone at the
front of the nested PTY, and SHALL type the launch for that shell. It SHALL
refuse the pane as busy when anything else leads the nested PTY.

#### Scenario: A wrapped shell at its prompt takes the launch

- **WHEN** the pane runs `atuin pty-proxy` hosting `zsh` at its prompt
- **THEN** `agent start` launches the agent through `zsh`

#### Scenario: A wrapped shell running a command is busy

- **WHEN** the shell behind the wrapper runs `vim`, or a subshell
- **THEN** `agent start` fails with `agent_pane_busy`
