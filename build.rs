/// build.rs — compile-time visualizer auto-discovery
///
/// Scans src/visualizers/*.rs (excluding mod.rs) and generates
/// $OUT_DIR/registry.rs containing:
///
///   - A `mod foo;` declaration for every discovered file
///   - A `pub fn all_visualizers() -> Vec<Box<dyn Visualizer>>` factory
///     that calls each file's `pub fn register() -> Vec<Box<dyn Visualizer>>`
///     and flattens the results.
///
/// src/visualizers/mod.rs includes that file with:
///   include!(concat!(env!("OUT_DIR"), "/registry.rs"));
///
/// CONTRACT: every file in src/visualizers/ (except mod.rs) must export:
///   pub fn register() -> Vec<Box<dyn Visualizer>>
///
/// A file that omits this will fail to compile with a clear linker error.
use std::fs;
use std::path::Path;

fn main() {
    let vis_dir = Path::new("src/visualizers");

    // Collect all .rs files that are not mod.rs
    let mut modules: Vec<String> = fs::read_dir(vis_dir)
        .expect("src/visualizers/ must exist")
        .filter_map(|e| {
            let entry = e.ok()?;
            let name = entry.file_name().into_string().ok()?;
            if name.ends_with(".rs") && name != "mod.rs" {
                Some(name.trim_end_matches(".rs").to_string())
            } else {
                None
            }
        })
        .collect();

    // Stable sort so the registry order is deterministic across platforms
    modules.sort();

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR must be set by cargo");
    let dest    = Path::new(&out_dir).join("registry.rs");

    let mut code = String::new();

    // One `pub mod foo;` per discovered file.
    //
    // Because this code is emitted into OUT_DIR/registry.rs and then
    // include!()-ed inside src/visualizers/mod.rs, plain `mod foo;` would
    // make rustc look for the source file relative to OUT_DIR -- where it
    // doesn't exist.  We use a `#[path = "..."]` attribute with the
    // absolute path to the real source file so rustc always finds it.
    //
    // `pub` is required so that main.rs can reach
    // `visualizers::spectrum::SpectrumViz::new(...)` etc.
    let src_dir = std::fs::canonicalize("src/visualizers")
        .expect("src/visualizers/ must exist and be canonicalisable");

    for m in &modules {
        let abs = src_dir.join(format!("{m}.rs"));
        // Use forward slashes even on Windows to keep the path valid in a
        // Rust string literal.
        let abs_str = abs.to_string_lossy().replace('\\', "/");
        code.push_str(&format!("#[path = \"{abs_str}\"]\npub mod {m};\n"));
    }

    code.push('\n');

    // Factory function: call register() on every module and flatten.
    code.push_str(
        "/// Return every built-in visualizer, in alphabetical file order.\n\
         /// Called once at startup by main.rs.\n\
         pub fn all_visualizers() -> Vec<Box<dyn Visualizer>> {\n\
             let mut out: Vec<Box<dyn Visualizer>> = Vec::new();\n",
    );
    for m in &modules {
        code.push_str(&format!("    out.extend({m}::register());\n"));
    }
    code.push_str(
        "    out\n\
         }\n",
    );

    fs::write(&dest, &code).expect("failed to write registry.rs");

    // Re-run only when a visualizer file is added or removed, or when
    // build.rs itself changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/visualizers");
}
