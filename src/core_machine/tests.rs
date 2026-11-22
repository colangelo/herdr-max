//! V23 (core-phase5 D9): herdr never installs or updates hc. An
//! architecture test over the source: no `machine.update` call, no
//! `--install` flag, and no `update` argument in any file that runs hc.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn herdr_never_installs_or_updates_hc() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let this = src.join("core_machine/tests.rs");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    // Spelled in parts so this file does not match itself.
    let update_method = ["machine", ".update"].concat();
    let install_flag = ["\"--", "install\""].concat();
    let update_arg = ["\"up", "date\""].concat();
    let runs_hc = [["hc_", "binary"].concat(), ["hc_", "command"].concat()];
    let mut found = Vec::new();
    for file in files.iter().filter(|file| **file != this) {
        let text = std::fs::read_to_string(file).expect("read");
        let reaches_hc = runs_hc.iter().any(|name| text.contains(name.as_str()));
        for (number, line) in text.lines().enumerate() {
            if line.contains(&update_method)
                || line.contains(&install_flag)
                || (reaches_hc && line.contains(&update_arg))
            {
                found.push(format!(
                    "{}:{}: {}",
                    file.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(files.len() > 100, "the scan found too few files");
    assert!(
        found.is_empty(),
        "herdr must not install or update hc:\n{}",
        found.join("\n")
    );
}
