//! Shrinks the GLSL sources before `include_str!` embeds them in the wasm:
//! comments and indentation are removed. Line numbers are kept, so shader
//! compile errors still point at the right line of `src/shaders/*`.
//! The `minify` function lives in `shader_minify.rs`, which the unit tests in
//! `src/render.rs` also include and check.

use std::{env, fs, path::Path};

include!("shader_minify.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/shaders");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=shader_minify.rs");
    let out_dir = Path::new(&env::var("OUT_DIR").expect("OUT_DIR is set by cargo")).join("shaders");
    fs::create_dir_all(&out_dir).expect("create shader output dir");
    for entry in fs::read_dir("src/shaders").expect("read src/shaders") {
        let path = entry.expect("read shader entry").path();
        let src = fs::read_to_string(&path).expect("read shader");
        let name = path.file_name().expect("shader file name");
        fs::write(out_dir.join(name), minify(&src)).expect("write shader");
    }
}
