use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=message.txt");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let input = fs::read_to_string("message.txt").expect("message.txt exists");
    let escaped = input.trim().replace('\\', "\\\\").replace('"', "\\\"");
    let output = format!("pub const GENERATED_MESSAGE: &str = \"{escaped}\";\n");

    fs::write(out_dir.join("generated.rs"), output).expect("generated output can be written");
}

