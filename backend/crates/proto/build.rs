//! Compiles every `.proto` under `<repo>/proto` into Rust.
//!
//! `buf build` produces a descriptor set (so buf.yaml stays the single source
//! of truth for the module layout) and `connectrpc-build` turns it into
//! buffa message types plus Connect RPC service traits and clients.
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

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

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // TOKULY_PROTO_ROOT lets container builds point at a copied proto tree.
    let repo_root = env::var("TOKULY_PROTO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("../../.."));
    let proto_dir = repo_root.join("proto");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let descriptor = out_dir.join("tokuly.binpb");

    println!("cargo:rerun-if-changed={}", proto_dir.display());
    println!("cargo:rerun-if-changed={}", repo_root.join("buf.yaml").display());
    println!("cargo:rerun-if-env-changed=TOKULY_PROTO_ROOT");

    let buf = env::var("BUF").unwrap_or_else(|_| "buf".into());
    let status = Command::new(&buf)
        .current_dir(&repo_root)
        .args(["build", "--as-file-descriptor-set", "-o"])
        .arg(&descriptor)
        .status()
        .unwrap_or_else(|e| panic!("failed to run `{buf}` (install buf: https://buf.build): {e}"));
    assert!(status.success(), "`buf build` failed");

    let mut files = Vec::new();
    collect(&proto_dir, &proto_dir, &mut files);

    connectrpc_build::Config::new()
        .descriptor_set(&descriptor)
        .files(&files)
        .emit_descriptor_set("tokuly_descriptor.bin")
        .include_file("_tokuly.rs")
        .compile()
        .expect("protobuf code generation failed");
}
