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
- Other clients (the phone app, a Herdr Mac app) write the same document, so herdr-max
  owns only the spaces it marks with `"herdr_max": 1`. A background thread reads the
  document, keeps every unmarked space as it is, puts herdr-max's current spaces in
  place of its marked ones, and writes with `doc.set` using compare-and-set on the
  version it read. It keeps only the latest spaces and skips spaces equal to the last
  ones it wrote. On `conflict` it reads and merges again. A value it cannot decode is
  left alone.
- The server builds the document on its main loop, 500 ms after a change that would
  also be saved to the session, and once at startup and after a handoff.
- Behind `[experimental] core_spaces_doc` (default false). Config reload applies it.
  Turning it on publishes the layout. Turning it off removes herdr-max's spaces and
  writes `null` only when nothing else remains.

## What does not change

- herdr's layout stays the only source of herdr's spaces. herdr never applies the
  document to its layout. A phone or Mac app's edit to one of herdr's spaces is replaced
  by herdr's next publish. Other clients' spaces are never changed.
- PTY panes: they are not in core, so the phone cannot show them. They are left out.
- Windows: the core backend is Unix only.

## Out of scope

- Seen and pinned labels (`herdr.seen`, `herdr.pinned`), tracked on #294.
- Applying an edit made in an app back to herdr's layout.
