# Preserve a session file that cannot be read

## Why

Measured on `0.8.0-ac-beta.79-evra`: truncate `session.json` to half its bytes,
start the server, and the session comes up empty — then, five seconds later, the
server writes a fresh empty session **over the damaged file**. The
partially-readable original, from which most panes and todos could have been
recovered by hand, is gone.

The empty startup is not the defect. Losing the evidence is. The maintainer's
live session file currently holds 48 todos across 9 panes — agent hand-off notes
that exist nowhere else.

The same silent overwrite happens on two other paths that already return "no
session": an unreadable file, and a file written by a newer herdr than the one
starting. That last one means downgrading destroys the session it declined to
read.

## What Changes

When the session file cannot be used, it is moved aside to a timestamped name
before anything else happens, and the move is logged. The session still starts
empty; the file it could not read is simply no longer at the path a save would
overwrite.

Session saves also flush the temporary file before renaming it into place. The
write was already atomic against a torn *write*; the missing flush is what lets
a power loss produce a torn *file* in the first place.

## Impact

- Affected capability: `session-persistence` (new)
- Affected code: `src/persist/io.rs` — the load failure paths and
  `save_json_to_path`
- Preserved files are never deleted automatically. They are rare, small, and are
  the last copy of something; reclaiming them is the user's call.
- No server, API, protocol or config surface

## Non-goals

- A rotating backup of *successful* loads. Moving the unusable file aside makes
  the measured loss impossible; a good-load backup guards a different risk
  (a valid but unwanted save) and would need a retention policy of its own.
  Revisit if that risk shows up.
- Recovering data out of a torn file automatically. Preserving it lets a human
  or a later tool do that; guessing at half a JSON document does not belong in
  the startup path.
- The save debounce window, which is filed separately.
