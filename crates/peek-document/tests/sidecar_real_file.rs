//! Reads a real sidecar from `~/peek` when one is present.
//!
//! The synthetic fixtures in `results_file.rs` prove the shape; this proves the shape is the one
//! the TypeScript app actually writes, against files of 3 KB to 13 MB. It skips silently when
//! `~/peek` is not there, so the suite still runs on a machine without Peek installed.

use std::path::PathBuf;

use peek_document::ResultSidecar;

fn sidecars() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let workspaces = PathBuf::from(home).join("peek").join("workspaces");
    let Ok(entries) = std::fs::read_dir(&workspaces) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|workspace| std::fs::read_dir(workspace.path()).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().ends_with(".results.json"))
        .collect()
}

#[test]
fn real_sidecars_parse_and_round_trip() {
    let paths = sidecars();
    if paths.is_empty() {
        eprintln!("no ~/peek sidecars found; skipping");
        return;
    }
    eprintln!("checking {} real sidecars", paths.len());
    for path in paths {
        let contents = std::fs::read_to_string(&path).expect("sidecar readable");
        let sidecar = ResultSidecar::from_json(&contents);
        assert!(
            !sidecar.is_empty(),
            "{} parsed to nothing; the sidecar shape has drifted",
            path.display()
        );

        // Re-reading what we write must give the same rows back, or autosave would corrupt the
        // file the TypeScript app is also reading.
        let reparsed = ResultSidecar::from_json(&sidecar.to_json());
        assert_eq!(
            reparsed,
            sidecar,
            "{} did not survive a write/read cycle",
            path.display()
        );
    }
}
