use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=../worker/src");
    println!("cargo::rerun-if-changed=../linkup/src");

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR to be set");

    let worker_target_dir = Path::new(&out_dir).join("worker-target");

    let install_status = Command::new("cargo")
        // TODO(augustoccesar)[2026-02-25]: From 0.7.0, the worker-build version is aligned
        //   with the version of the worker crate. When bumping the worker, it needs to also
        //   bump the worker-build.
        .args(["install", "-q", "worker-build@0.8.7"])
        .current_dir("../worker")
        .env("CARGO_TARGET_DIR", &worker_target_dir)
        .status()
        .expect("failed to execute worker-build install process");

    if !install_status.success() {
        panic!("Failed to install worker-build");
    }

    let build_status = Command::new("worker-build")
        .args(["--release"])
        .current_dir("../worker")
        .env("CARGO_TARGET_DIR", &worker_target_dir)
        .status()
        .expect("failed to execute worker-build process");

    if !build_status.success() {
        panic!("Failed to build worker");
    }

    let index_js_src = Path::new("../worker/build/index.js");
    let wasm_src = Path::new("../worker/build/index_bg.wasm");
    let index_js_dest = Path::new(&out_dir).join("index.js");
    let wasm_dest = Path::new(&out_dir).join("index_bg.wasm");

    fs::create_dir_all(&out_dir).expect("failed to create output directories");

    fs::copy(index_js_src, &index_js_dest).expect("failed to copy index.js");
    fs::copy(wasm_src, &wasm_dest).expect("failed to copy index_bg.wasm");
}
