//! Architecture test for the core backend (openspec/changes/core-backend,
//! design.md D6 and V13): herdr never asks herdr-core to type for it. Core's
//! `terminal.send` waits for a person's unfinished draft and then pastes onto
//! it (phase 0 Q13); every input goes on the pane's own attach stream instead.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn nothing_calls_core_terminal_send() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let mut found: Vec<String> = files
        .iter()
        .filter(|path| {
            fs::read_to_string(path)
                .expect("read source file")
                .contains("\"terminal.send\"")
        })
        .map(|path| {
            path.strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string()
        })
        .collect();
    found.sort();
    assert!(
        found.is_empty(),
        "these files call herdr-core's terminal.send; write to the pane's attach stream instead: {found:?}"
    );
}
