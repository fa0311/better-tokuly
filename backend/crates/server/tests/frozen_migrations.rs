//! Released migrations never change: a deployment that applied one refuses
//! to start when its checksum differs (sqlx compares SHA-384). Every file in
//! `backend/migrations/frozen.sha384` (written at each stable release, in
//! `sha384sum` format) must still match it.
use std::path::PathBuf;

use sha2::{Digest, Sha384};

#[test]
fn released_migrations_are_unchanged() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let table = std::fs::read_to_string(dir.join("frozen.sha384")).expect("backend/migrations/frozen.sha384 is readable");
    let mut changed = Vec::new();
    for line in table.lines().filter(|line| !line.trim().is_empty()) {
        let (sum, file) = line.split_once("  ").unwrap_or_else(|| panic!("not a `sha384sum` line: {line:?}"));
        let sql = std::fs::read(dir.join(file)).unwrap_or_else(|e| panic!("{file} is released and cannot go away: {e}"));
        if hex::encode(Sha384::digest(&sql)) != sum {
            changed.push(file);
        }
    }
    assert!(
        changed.is_empty(),
        "released migrations were edited: {changed:?}. Put the change in a new file of the domain's range instead (backend/README.md, \"After a release\")"
    );
}
