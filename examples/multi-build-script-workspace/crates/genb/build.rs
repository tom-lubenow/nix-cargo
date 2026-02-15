use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=b.txt");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let message = fs::read_to_string(manifest_dir.join("b.txt")).expect("b.txt exists");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set"));
    let generated = out_dir.join("generated.rs");
    fs::write(
        generated,
        format!("pub const B_MSG: &str = {:?};", message.trim()),
    )
    .expect("write generated.rs");
}
