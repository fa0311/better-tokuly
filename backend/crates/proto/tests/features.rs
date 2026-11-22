//! The feature table in `Cargo.toml` follows the proto tree: one feature per
//! package, depending on exactly the packages its `.proto` files import.
//!
//! A build that enables a package without what it imports fails in `build.rs`,
//! but only the combinations somebody builds are ever tried; this checks the
//! whole table.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

type Table = BTreeMap<String, BTreeSet<String>>;

/// The `[features]` table of this crate's manifest (one feature per line).
fn manifest_features() -> Table {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let section = manifest.split("\n[features]\n").nth(1).expect("Cargo.toml has a [features] table");
    let section = section.split("\n[").next().unwrap();
    section
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, list) =
                line.split_once(" = [").unwrap_or_else(|| panic!("keep each feature on one line: `{line}`"));
            let list = list.strip_suffix(']').unwrap_or_else(|| panic!("keep each feature on one line: `{line}`"));
            let enables =
                list.split(',').map(|item| item.trim().trim_matches('"').to_owned()).filter(|item| !item.is_empty());
            (name.to_owned(), enables.collect())
        })
        .collect()
}

fn proto_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().map(Result::unwrap) {
        let path = entry.path();
        if path.is_dir() {
            proto_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "proto") {
            out.push(path);
        }
    }
}

/// Every package under `proto/tokuly` with the packages its files import.
fn proto_packages() -> Table {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../proto/tokuly");
    let mut packages = Table::new();
    for entry in fs::read_dir(&root).unwrap().map(Result::unwrap).filter(|entry| entry.path().is_dir()) {
        let package = entry.file_name().to_string_lossy().into_owned();
        let mut files = Vec::new();
        proto_files(&entry.path(), &mut files);
        let imports = files
            .iter()
            .flat_map(|file| fs::read_to_string(file).unwrap().lines().map(str::to_owned).collect::<Vec<_>>())
            .filter_map(|line| {
                line.trim().strip_prefix("import \"tokuly/").and_then(|rest| rest.split('/').next().map(str::to_owned))
            })
            .filter(|import| *import != package)
            .collect();
        packages.insert(package, imports);
    }
    packages
}

#[test]
fn the_feature_table_mirrors_the_proto_imports() {
    let mut features = manifest_features();
    assert!(!features.contains_key("default"), "no default features: every crate says which packages it uses");
    assert_eq!(
        features.remove("client"),
        Some(BTreeSet::new()),
        "`client` gates the generated clients and enables no package"
    );
    let packages = proto_packages();
    for (package, imports) in &packages {
        let enabled =
            features.get(package).unwrap_or_else(|| panic!("proto package `{package}` has no feature in Cargo.toml"));
        assert_eq!(
            enabled, imports,
            "feature `{package}` must list exactly the packages proto/tokuly/{package} imports"
        );
    }
    let unknown: Vec<&String> = features.keys().filter(|feature| !packages.contains_key(*feature)).collect();
    assert!(unknown.is_empty(), "features without a proto package: {unknown:?}");
}
