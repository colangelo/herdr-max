## 1. Plan

- [x] 1.1 This change, validated with `openspec validate core-spaces-doc --strict`

## 2. Document

- [x] 2.1 `SpacesDocument` types and `spaces_document` mapping in `src/app/core_spaces_doc.rs`, tested against a copy of the app's shapes (verifier 1-4)

## 3. Publisher

- [x] 3.1 `DocPublisher` / `DocStore` / `CoreDocStore` and the publisher thread in `core_backend/spaces_doc.rs`, facade `crate::pane::publish_core_spaces` (verifier 5-8)

## 4. Server

- [x] 4.1 `[experimental] core_spaces_doc` config, test and `config-reference.json` entry (verifier 9)
- [x] 4.2 `core_spaces_dirty`, `sync_core_spaces_schedule`, deadline in the server loop, startup/handoff/reload (verifier 9)

## 5. Live

- [x] 5.1 Live core test: publish, read back, conflict, clear (verifier 10-11); server path in `scripts/core_live_test.sh` (verifier 13)
- [x] 5.2 `just check` and `just core-live-test` green
- [x] 5.4 herdr-max owns only its marked spaces: `merge` in `spaces_doc.rs`, writer reads the value, re-merges on conflict, leaves an undecodable value; live tests seed a foreign space (verifier 5-8, 10-11, 13-18); `just check` and `just core-live-test` green
- [ ] 5.3 Beta with the flag on in ac's m4m config; ac checks the phone's spaces (verifier 12)
