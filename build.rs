use std::{env, path::PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));

    prost_build::Config::new()
        .type_attribute(".", "#[allow(dead_code)]")
        .file_descriptor_set_path(out_dir.join("liqi_desc.bin"))
        .compile_protos(&["proto/liqi.proto"], &["proto/"])
        .expect("failed to compile liqi.proto");

    println!("cargo:rerun-if-changed=proto/liqi.proto");
    println!("cargo:rerun-if-changed=build.rs");

    tauri_build::build();
}
