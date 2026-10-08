## 1. Preserve

- [x] 1.1 Move an unusable session file aside under a UTC-timestamped name, logging the reason and the new path
- [x] 1.2 Use it on all three paths that decline a file: unreadable, unparseable, newer snapshot version
- [x] 1.3 Preserve by renaming, so the file is never both unusable and still at the save path

## 2. Durability

- [x] 2.1 Flush the temporary file before the rename in `save_json_to_path`

## 3. Tests

- [x] 3.1 A torn file is preserved with its contents intact, and the load returns no session
- [x] 3.2 A newer-version file is preserved rather than left in place
- [x] 3.3 Two failures in a row leave two distinct preserved files
- [x] 3.4 A good file is loaded and left where it is

## 4. Verification

- [x] 4.1 `just check` green
- [x] 4.2 Re-run the torn-file probe against the built binary and confirm the file survives
