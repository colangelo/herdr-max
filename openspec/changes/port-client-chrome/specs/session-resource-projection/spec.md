## Purpose

Provide clients with optional session resource facts for presentation while retaining interoperability with the published endpoint compatibility floor.

## ADDED Requirements

### Requirement: Optional shared resource facts
Endpoint snapshot JSON SHALL optionally expose shared resource facts keyed by stable resource IDs. Facts SHALL belong to the same endpoint boot and revision as their core snapshot. Missing facts SHALL preserve baseline rendering and disable only unsupported feature actions.

#### Scenario: Older endpoint
- **WHEN** a client receives a baseline snapshot without resource facts
- **THEN** it retains a compatible connection and baseline presentation

#### Scenario: Resource identities collide across endpoints
- **WHEN** two endpoints use the same resource ID
- **THEN** each fact and action remains scoped to its originating endpoint

### Requirement: Published core codecs remain immutable
Optional fact projection SHALL preserve the byte shape and meaning of all published core codecs and baseline required JSON fields. New actions SHALL use separately advertised methods without changing existing method parameters.

#### Scenario: Legacy binary snapshot
- **WHEN** shared facts accompany an endpoint JSON snapshot
- **THEN** encoding its baseline snapshot through a published binary codec remains identical to encoding without those facts

### Requirement: Malformed optional facts do not reject baseline
A client SHALL decode the baseline independently of optional facts. Invalid or unknown optional fact values SHALL fall back without rejecting the compatible baseline or disconnecting other endpoints.

#### Scenario: Invalid optional facts
- **WHEN** otherwise valid baseline JSON includes malformed optional facts
- **THEN** the client accepts the baseline and omits those unsupported facts

### Requirement: Canonical pane labels remain cached resource facts
Optional pane name facts SHALL retain the fork's cached label precedence: reported title, assigned label, terminal title, agent name or kind, launch command, and stable pane number. Their computation SHALL NOT inspect runtime terminal buffers or processes during client rendering. They SHALL NOT reinterpret the baseline pane label field.

#### Scenario: Pane terminal state is unavailable
- **WHEN** a known pane temporarily has no terminal state
- **THEN** its projected name retains its stable pane-number fallback

#### Scenario: Older endpoint lacks canonical names
- **WHEN** an endpoint provides no pane name facts
- **THEN** the client uses available baseline title and label metadata without rejecting the endpoint

### Requirement: Version labels identify the connected server
Clients SHALL retain the welcome server version as the compatibility-floor version source. A coherent current server-version fact SHALL replace it for presentation after a live handoff, even when the client socket remains attached.

#### Scenario: Live handoff preserves a client connection
- **WHEN** a newer server advertises its version in the matching snapshot facts
- **THEN** pane-label summaries identify that server build rather than the client's earlier welcome build
