## Decisions

### `name` carries the fallback, with a marker

`agent list` consumers (CONTEXT's idle maintenance, scripts) read `name`.
Filling it with the fallback and adding `name_source: "title"` makes them
work unchanged, and a consumer that must tell the two apart reads the marker.
A separate field would leave every consumer blind until it is updated.

### The name rule

A terminal title is a name only when it is one word of 1-64 characters from
`[A-Za-z0-9._-]`, starting with a letter or digit, and is not an agent kind
(`crate::detect::identify_agent`) or a pane shell name. The space rule removes
Claude's default "Claude Code" and every task sentence; the character set
removes paths and prompts (`~/src`, `user@host`). This is strict on purpose: a
wrong name is worse than none, because tools act on it.

### Uniqueness

Explicit names are unique already (`agent rename` refuses a taken name). A
fallback equal to an explicit name elsewhere, or shared by two agents, is
dropped for all of them, so a name always resolves to one agent. An explicit
rename only checks explicit names: taking a name another agent had as its
fallback just removes that fallback.

### Computed on demand

Names are resolved when an API call or a notification needs them, from the
live title, with one pass over the terminals. Nothing is stored, so there is
nothing to keep in sync with `/rename`, restore or handoff.

### Notification title

`<name> <event>` where name is explicit, else fallback, else the agent label
as today. The mobile banner strips the event suffix, which is unchanged.
