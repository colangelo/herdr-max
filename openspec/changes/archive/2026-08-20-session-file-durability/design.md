# Design

## Preserving and protecting are one act

The obvious shape is "copy the file, then carry on". Renaming is better: after a
copy the original is still sitting at the path the next save writes to, so the
protection depends on the copy having happened first and on nothing else
touching the path in between. A rename removes the file from harm's way and
preserves it in the same syscall — there is no window to reason about.

It also means the next start finds no session file at all rather than the same
unusable one, so a machine that fails this way twice does not accumulate the
same complaint every boot.

## Timestamped, and never swept

`session.<YYYYMMDD-HHMMSS>Z.bak.json`, UTC. Sortable by name, readable at a
glance, and distinct across repeated failures — a counter would need to read the
directory to know where it got to, and a plain `.bak` would let the second
failure destroy what the first preserved, which is the whole bug again one level
up.

Nothing deletes these. They are rare, a few tens of kilobytes, and each one is
the last copy of a session someone lost; a cleanup policy would be inventing a
reason to throw away the only thing that survived.

## Why the version path is included

An unreadable or unparseable file is obviously a failure. A file from a newer
herdr is not — it is declined deliberately. But the consequence is identical: a
save follows and overwrites it. Downgrading herdr, or running an older binary
once, would silently destroy the newer session. Same fix, same call site.

## fsync before rename

`save_json_to_path` already writes a temp file and renames it, which is atomic
against a torn *write*. It does not flush, so on a power loss the rename can be
durable before the data is — which is precisely how a half-written file appears
at the session path. Flushing before the rename closes that, and costs one
`sync_all` per save, at most once per five seconds.
