// Shared by `build.rs` (via `include!`) and the tests in `src/render.rs`, so
// the tested function is the one that runs at build time.

/// Removes `//` comments and indentation from GLSL, one output line per
/// input line so compile-error line numbers still match the source.
fn minify(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let code = line.split("//").next().unwrap_or("").trim();
        out.push_str(code);
        out.push('\n');
    }
    out
}
