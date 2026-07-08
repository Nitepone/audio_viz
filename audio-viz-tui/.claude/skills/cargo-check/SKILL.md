---
name: cargo-check
description: Run `cargo check --lib` and report any errors or warnings. Use after any edit to Rust source files to confirm the build is clean.
---

Run `cargo check --lib` from the repository root and report the result.

## What to do

1. Run `cargo check --lib` via Bash.

2. If it **succeeds with no warnings**, say: "Clean — no errors or warnings."

3. If it **succeeds with warnings**, list each warning with its file, line number, and message. Ask the user if they want them fixed.

4. If it **fails**, list each error with its file, line number, and message, then fix them without prompting. After fixing, run `cargo check --lib` again to confirm clean.

## Important

- Always use `--lib` (not `--release` or default). This checks the shared library which covers all visualizer code and excludes terminal-only crates.
- Never run the WASM build (`wasm-pack`) — terminal only.
