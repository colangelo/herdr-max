https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/141

## 1. State

- [ ] 1.1 tests first: membership, exclusion, new pane joins, closed pane leaves, off with the last pane gone
- [ ] 1.2 `Tab.sync: Option<SyncPanes>`, helpers `set_sync`, `toggle_pane_sync`, invariants; not in snapshots

## 2. Fan-out

- [ ] 2.1 tests first: key, text commit and paste reach every synced runtime, exclusion honoured, prefix chord not sent, other tabs untouched
- [ ] 2.2 `sync_targets` helper and the three input paths, per-target encoding

## 3. Mouse and menu

- [ ] 3.1 tests first: right click toggles membership while syncing, opens the menu otherwise; menu item "Sync input: on/off"
- [ ] 3.2 mouse handling and `ContextMenuKind::Pane` item

## 4. Visual hint

- [ ] 4.1 tests first: yellow borders for synced panes, none for an excluded one, yellow bottom bar chip
- [ ] 4.2 palette entry and render changes

## 5. API, CLI, key

- [ ] 5.1 `tab.sync`, `pane.sync`, `sync` and `synced` fields, CLI, schema artifact
- [ ] 5.2 `toggle_sync_panes` action, default prefix+shift+s, help entry; docs, `just check`

## 6. Verify

- [ ] 6.1 throwaway server: three panes, type, exclude one by right click, type, captures on the issue

## 7. Round 2 (issue 155)

- [ ] 7.1 tests first: explicit members (pair start, whole-tab start, toggle, new pane joins only a whole-tab group), empty group then grace then off, re-add cancels, group of one
- [ ] 7.2 `SyncPanes` members/whole_tab/ending_until, `sync_deadline`/`expire_sync` in the loop, `SYNC ending…`
- [ ] 7.3 `#FFD60A` borders and chip, gray border for panes outside the group
- [ ] 7.4 "Sync input" on another pane starts the pair; focused pane and the key stay whole-tab; right click re-adds during the grace
- [ ] 7.5 API/CLI semantics, docs, `just check`, throwaway proof with captures
