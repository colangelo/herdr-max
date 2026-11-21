# Publish herdr's workspaces as Herdr app spaces

Tracking issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/293

## Why

ac uses the Herdr Mobile iPhone app with herdr-max on m4m. The phone lists herdr-max's
core terminals, and its Inbox shows agent status (`core_status_mirror`). It shows no
spaces. The Herdr apps read spaces from one herdr-core document per machine, named
`herdr-app`, and today only the Herdr Mac app writes it. herdr-max writes none, so the
phone has terminals but no workspaces, tabs or splits to put them in.

## What changes

- The herdr server builds a `herdr-app` document from its own layout. Each workspace
  becomes a space (`id`, `name`, `folder`), each tab becomes a tab, and each tab's split
  tree becomes the app's layout `Node`. Each pane that runs in herdr-core becomes a
  `terminal` block that holds the pane's core address.
- A background thread writes the document into this herdr session's core session with
  `doc.set`, using compare-and-set on the version. It keeps only the latest document.
  It skips a document equal to the last one it wrote. On `conflict` it reads the
  current version and writes again.
- The server builds the document on its main loop, 500 ms after a change that would
  also be saved to the session, and once at startup and after a handoff.
- Behind `[experimental] core_spaces_doc` (default false). Config reload applies it.
  Turning it on publishes the layout. Turning it off writes `null` to the document.

## What does not change

- herdr's layout stays the only source. herdr never reads the document back. A space
  that a phone or Mac app edits is overwritten by herdr's next publish.
- PTY panes: they are not in core, so the phone cannot show them. They are left out.
- Windows: the core backend is Unix only.

## Out of scope

- Seen and pinned labels (`herdr.seen`, `herdr.pinned`), tracked on #294.
- Applying an edit made in an app back to herdr's layout.
