//! Architecture test for the pane backend seam (openspec/changes/runtime-seam,
//! design.md § Verifier, V9): only the seam and its allowed neighbours name the
//! PTY layer, so a new spawn path cannot bypass it.

use std::fs;
use std::path::{Path, PathBuf};

/// Files that may name `crate::pty::`. The `src/pty/` tree and the seam,
/// `src/pane/runtime_io/`, are always allowed.
const PTY_LAYER_ALLOWED: &[&str] = &[
    // Test-only since runtime-seam task 4.6: `PaneRuntime::test_with_actor` and
    // the handoff import test in `src/pane.rs`, and `TerminalRuntime::
    // test_with_actor` in `src/terminal/runtime.rs`. Production code reaches
    // the PTY layer only through `src/pane/runtime_io/`.
    "src/pane.rs",
    "src/terminal/runtime.rs",
    // Tests that build PTY types directly (runtime-seam roundtable F4).
    "src/pane/runtime_characterization_tests.rs",
    "src/platform/macos/input_tests.rs",
];

/// Files that may name `portable_pty`, besides `src/pty/`, `src/platform/` and
/// the seam.
const PORTABLE_PTY_ALLOWED: &[&str] = &[
    // Command builders (design D5): `CommandBuilder` is how a spawn is described.
    "src/pane.rs",
    "src/integration/env.rs",
    // Tests that open a PTY themselves.
    "src/detect/mod.rs",
    "src/pane/runtime_characterization_tests.rs",
    "src/pane/terminal/migration_tests.rs",
    "src/server/headless/tests/mod.rs",
];

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

fn offenders(needle: &str, always: &[&str], allowed: &[&str]) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let mut found: Vec<String> = files
        .iter()
        .filter_map(|path| {
            let rel = path
                .strip_prefix(root)
                .expect("under the manifest dir")
                .to_string_lossy()
                .replace('\\', "/");
            let inside = always.iter().any(|prefix| rel.starts_with(prefix));
            let listed = allowed.contains(&rel.as_str());
            let text = fs::read_to_string(path).expect("read source file");
            (!inside && !listed && text.contains(needle)).then_some(rel)
        })
        .collect();
    found.sort();
    found
}

#[test]
fn only_the_seam_names_the_pty_layer() {
    let found = offenders(
        "crate::pty::",
        &["src/pty/", "src/pane/runtime_io/"],
        PTY_LAYER_ALLOWED,
    );
    assert!(
        found.is_empty(),
        "these files reach into crate::pty:: outside the pane backend seam: {found:?}"
    );
}

#[test]
fn only_the_seam_and_command_builders_name_portable_pty() {
    let found = offenders(
        "portable_pty",
        &["src/pty/", "src/platform/", "src/pane/runtime_io/"],
        PORTABLE_PTY_ALLOWED,
    );
    assert!(
        found.is_empty(),
        "these files name portable_pty outside the pane backend seam: {found:?}"
    );
}

#[test]
fn every_allowed_file_still_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for rel in PTY_LAYER_ALLOWED.iter().chain(PORTABLE_PTY_ALLOWED) {
        assert!(
            root.join(rel).is_file(),
            "{rel} is allowed but gone: remove it from the list"
        );
    }
}
