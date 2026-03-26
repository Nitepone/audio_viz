/// build.rs — compile-time visualizer auto-discovery
///
/// Scans src/visualizers/<category>/*.rs and generates
/// $OUT_DIR/registry.rs containing:
///
///   - A `pub mod foo;` declaration for every discovered file (using
///     an absolute #[path] so rustc can find the file regardless of
///     where registry.rs lives).
///   - A `pub fn all_visualizers() -> Vec<Box<dyn Visualizer>>` factory
///     that calls each file's `register()` and flattens the results.
///   - A `pub fn visualizer_categories() -> Vec<(&'static str, Vec<&'static str>)>`
///     that returns the category name and sorted visualizer names within.
///
/// src/visualizers/mod.rs includes that file with:
///   include!(concat!(env!("OUT_DIR"), "/registry.rs"));
///
/// CONTRACT: every .rs file inside a subdirectory of src/visualizers/
/// (except mod.rs) must export:
///   pub fn register() -> Vec<Box<dyn Visualizer>>
///
/// To add a new visualizer:
///   1. Create src/visualizers/<category>/mything.rs
///   2. Implement the Visualizer trait
///   3. Export: pub fn register() -> Vec<Box<dyn Visualizer>> { ... }
///   4. Run:    cargo build
use std::fs;
use std::path::Path;

struct VisModule {
    /// Module / basename name (e.g. "spectrum")
    name: String,
    /// Category directory name (e.g. "frequency")
    category: String,
    /// Absolute path to the source file, forward-slash normalised
    abs_path: String,
}

fn main() {
    let vis_dir = Path::new("src/visualizers");
    let src_dir = std::fs::canonicalize(vis_dir)
        .expect("src/visualizers/ must exist and be canonicalisable");

    let mut modules: Vec<VisModule> = Vec::new();

    // Scan one level of subdirectories only; skip root .rs files (mod.rs lives there).
    for entry in fs::read_dir(vis_dir).expect("src/visualizers/ must exist") {
        let entry = entry.expect("failed to read entry in src/visualizers/");
        let path  = entry.path();
        if !path.is_dir() { continue; }

        let category = path
            .file_name()
            .expect("directory with no name")
            .to_string_lossy()
            .to_string();

        for sub in fs::read_dir(&path)
            .unwrap_or_else(|_| panic!("failed to read src/visualizers/{}/", category))
        {
            let sub  = sub.expect("failed to read sub-entry");
            let name = sub.file_name().into_string().expect("non-UTF-8 filename");
            if !name.ends_with(".rs") || name == "mod.rs" { continue; }

            let mod_name = name.trim_end_matches(".rs").to_string();
            let abs_path = src_dir
                .join(&category)
                .join(&name)
                .to_string_lossy()
                .replace('\\', "/");

            modules.push(VisModule { name: mod_name, category: category.clone(), abs_path });
        }
    }

    // Stable sort: category then name, so output is deterministic across platforms.
    modules.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR must be set by cargo");
    let dest    = Path::new(&out_dir).join("registry.rs");
    let mut code = String::new();

    // ── Module declarations ────────────────────────────────────────────────────
    //
    // Each `pub mod foo;` uses a `#[path]` attribute pointing to the real
    // source file so rustc can find it from OUT_DIR.  `pub` is required so
    // main.rs can reach e.g. `visualizers::spectrum::SpectrumViz`.
    for m in &modules {
        code.push_str(&format!(
            "#[path = \"{}\"]\npub mod {};\n",
            m.abs_path, m.name
        ));
    }
    code.push('\n');

    // ── all_visualizers() ─────────────────────────────────────────────────────
    code.push_str(
        "/// Return every built-in visualizer, sorted by category then name.\n\
         /// Called once at startup by main.rs.\n\
         pub fn all_visualizers() -> Vec<Box<dyn Visualizer>> {\n\
             let mut out: Vec<Box<dyn Visualizer>> = Vec::new();\n",
    );
    for m in &modules {
        code.push_str(&format!("    out.extend({}::register());\n", m.name));
    }
    code.push_str("    out\n}\n\n");

    // ── visualizer_categories() ───────────────────────────────────────────────
    //
    // Returns (category_name, [visualizer_name, ...]) pairs in sorted order.
    // Used by the in-app picker for the two-level category → visualizer menu.
    code.push_str(
        "/// Return visualizer names grouped by category, both sorted alphabetically.\n\
         pub fn visualizer_categories() -> Vec<(&'static str, Vec<&'static str>)> {\n\
             vec![\n",
    );

    // Collect unique categories in sorted order.
    let mut categories: Vec<&str> = Vec::new();
    for m in &modules {
        if !categories.contains(&m.category.as_str()) {
            categories.push(&m.category);
        }
    }

    for cat in &categories {
        let names: Vec<&str> = modules
            .iter()
            .filter(|m| m.category.as_str() == *cat)
            .map(|m| m.name.as_str())
            .collect();

        code.push_str(&format!("        (\"{}\", vec![", cat));
        for (i, n) in names.iter().enumerate() {
            if i > 0 { code.push_str(", "); }
            code.push_str(&format!("\"{}\"", n));
        }
        code.push_str("]),\n");
    }

    code.push_str("    ]\n}\n");

    fs::write(&dest, &code).expect("failed to write registry.rs");

    // Re-run whenever a visualizer file is added/removed, or build.rs changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/visualizers");
}
