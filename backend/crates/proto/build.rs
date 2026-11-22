//! Compiles the `.proto` packages this build asked for into Rust.
//!
//! Every directory `<repo>/proto/tokuly/<package>` has a Cargo feature of the
//! same name. `buf build` produces a descriptor set of the enabled packages
//! (so buf.yaml stays the single source of truth for the module layout) and
//! `connectrpc-build` turns it into buffa message types plus Connect RPC
//! service traits and, behind the `client` feature, clients.
//!
//! Only the enabled packages are inputs of the build: editing a `.proto` of
//! a package nobody in a build uses recompiles nothing in that build.
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

/// Features of this crate that are not proto packages.
const OTHER_FEATURES: &[&str] = &["client"];

/// The proto packages whose Cargo feature is enabled, sorted.
fn enabled_packages() -> Vec<String> {
    let mut packages: Vec<String> = env::vars()
        .filter_map(|(name, _)| name.strip_prefix("CARGO_FEATURE_").map(str::to_ascii_lowercase))
        .filter(|feature| !OTHER_FEATURES.contains(&feature.as_str()))
        .collect();
    packages.sort();
    packages
}

/// The `.proto` files below `dir`, as paths relative to `root`, sorted.
fn collect(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "proto") {
            let rel = path.strip_prefix(root).unwrap();
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// The packages a `.proto` file imports (`import "tokuly/<package>/...";`).
fn imports(file: &Path) -> Vec<String> {
    let source = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
    source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("import \"tokuly/"))
        .filter_map(|rest| rest.split('/').next())
        .map(str::to_owned)
        .collect()
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // TOKULY_PROTO_ROOT lets container builds point at a copied proto tree.
    let repo_root = env::var("TOKULY_PROTO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("../../.."));
    let proto_dir = repo_root.join("proto");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let descriptor = out_dir.join("tokuly.binpb");

    // What this script reads: buf.yaml, the directories of the enabled
    // packages (below) and a few variables. Nothing else may be declared: see
    // `emit_rerun_directives` at the end.
    println!("cargo:rerun-if-changed={}", repo_root.join("buf.yaml").display());
    println!("cargo:rerun-if-env-changed=TOKULY_PROTO_ROOT");
    println!("cargo:rerun-if-env-changed=BUF");
    // The generator's limit on descriptor size (for very large schemas).
    println!("cargo:rerun-if-env-changed=BUFFA_ELEMENT_MEMORY_LIMIT");

    let packages = enabled_packages();
    let mut files = Vec::new();
    for package in &packages {
        let dir = proto_dir.join("tokuly").join(package);
        assert!(dir.is_dir(), "feature `{package}` has no proto package: {} does not exist", dir.display());
        println!("cargo:rerun-if-changed={}", dir.display());
        let first = files.len();
        collect(&dir, &proto_dir, &mut files);
        // The generated code of a package names the types it imports, so
        // those packages have to be part of the build as well.
        for file in &files[first..] {
            for import in imports(&proto_dir.join(file)) {
                assert!(
                    packages.contains(&import),
                    "{file} imports the `{import}` package: add \"{import}\" to the `{package}` feature in backend/crates/proto/Cargo.toml"
                );
            }
        }
    }

    if packages.is_empty() {
        // Nothing asked for: `lib.rs` still includes these two.
        std::fs::write(out_dir.join("_tokuly.rs"), "").expect("cannot write to OUT_DIR");
        std::fs::write(out_dir.join("tokuly_descriptor.bin"), "").expect("cannot write to OUT_DIR");
        return;
    }

    let buf = env::var("BUF").unwrap_or_else(|_| "buf".into());
    let mut build = Command::new(&buf);
    build.current_dir(&repo_root).args(["build", "--as-file-descriptor-set", "-o"]).arg(&descriptor);
    for package in &packages {
        // Limits the build to these packages and whatever they import.
        build.arg("--path").arg(format!("proto/tokuly/{package}"));
    }
    let status = build.status().unwrap_or_else(|e| panic!("failed to run `{buf}` (install buf: https://buf.build): {e}"));
    assert!(status.success(), "`buf build` failed");

    connectrpc_build::Config::new()
        .descriptor_set(&descriptor)
        // The generator would declare the descriptor set as an input of the
        // build. This script has just written it, so cargo would find it
        // newer than the build on every run, and recompile this crate and
        // everything above it each time cargo is invoked.
        .emit_rerun_directives(false)
        .files(&files)
        .emit_descriptor_set("tokuly_descriptor.bin")
        .include_file("_tokuly.rs")
        // Nothing in the workspace calls the API as a Connect client, so the
        // generated clients are compiled only on request.
        .gate_client_feature(true)
        .compile()
        .expect("protobuf code generation failed");
}
