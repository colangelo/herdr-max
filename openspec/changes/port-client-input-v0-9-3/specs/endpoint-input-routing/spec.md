# Endpoint input routing

## Purpose

Keep Herdr Max keyboard and mouse workflows consistent across client-owned local and remote shells without weakening the published endpoint compatibility contract.

## ADDED Requirements

### Requirement: Client command ownership
The client SHALL consume prefix, navigation, copy and overlay commands locally and dispatch shared runtime mutations only through advertised methods. Every configurable action SHALL appear in effective keybinding help, including unbound actions.

#### Scenario: Multiple prefixes and indexed actions
- **WHEN** a client uses `prefix = ["ctrl+s", "ctrl+;"]` and indexed letter bindings
- **THEN** either prefix enters the same command mode and the matching visible target is selected without typing the command into the pane

#### Scenario: Optional method unavailable
- **WHEN** a runtime does not advertise an action's method
- **THEN** only that action is unavailable and other compatible endpoints remain connected

### Requirement: Input lifecycle ownership
Consumed commands SHALL repeat only in their intended client context. Forwarded press, repeat and release events SHALL retain their original runtime target. Entering copy or SCROLL through a held scroll gesture SHALL transfer repeats to that mode without repeating unrelated entry actions.

#### Scenario: Context changes during a held command
- **WHEN** an overlay closes or the scroll target changes while a key is held
- **THEN** its remaining repeats do not become terminal text or act on a different pane

#### Scenario: Copy viewport scrolling
- **WHEN** Ctrl+k or Ctrl+j scrolls the copy viewport
- **THEN** its absolute cursor stays on the same buffer row while visible and otherwise clamps to the viewport edge

### Requirement: Application scroll isolation
SCROLL SHALL target one alternate-screen pane and SHALL remain distinct from typed sync fanout. Unsupported or no-longer-alternate-screen wheel intents SHALL be dropped. Synthesized keys SHALL use press-only legacy encoding and matched press/release when the destination requests release events.

#### Scenario: Claude top and bottom
- **WHEN** SCROLL receives g, G or Ctrl+g for an effectively known Claude pane
- **THEN** it sends Ctrl+Home or Ctrl+End to that pane and leaves its prompt text unchanged

#### Scenario: Alternate screen lost before execution
- **WHEN** a queued line-scroll intent reaches a pane that has left the alternate screen
- **THEN** the intent does not scroll host history or type an arrow into prompt history

### Requirement: Synced typed input
Typed keys, committed text and paste SHALL reach each member of the origin pane's sync group exactly once, encoded for that destination. An excluded origin SHALL type alone. Popup, mouse and generated SCROLL input SHALL not fan out through this path.

#### Scenario: Heterogeneous peers
- **WHEN** synced destinations use different keyboard and bracketed-paste modes
- **THEN** each receives the encoding its own protocol requests rather than the focused pane's bytes

#### Scenario: Membership changes with held input
- **WHEN** sync membership or focus changes after a typed press
- **THEN** its matching releases reach the original surviving recipients without introducing a press on a new recipient

### Requirement: Compatible runtime extensions
New runtime input operations SHALL be advertised separately, with optional revision-bound feature projections. Generation-1 codecs, required baseline fields and existing endpoint method behavior SHALL remain unchanged. Scrollback-only clearing SHALL have a method distinct from screen clearing.

#### Scenario: Older endpoint
- **WHEN** an endpoint lacks sync, pin or application-scroll support
- **THEN** the corresponding controls remain unavailable without replacing or rejecting the baseline connection

#### Scenario: Clear saved history
- **WHEN** the scrollback-only action executes
- **THEN** saved history is purged while the entire visible screen, cursor and terminal modes remain intact
