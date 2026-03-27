---
name: update-viz-index
description: Update the `// ── Index:` comment at the top of a visualizer file after edits that shifted line numbers. Use after any edit to a visualizer that added or removed lines.
argument-hint: <path/to/visualizer.rs>
---

Update the file index comment in `$ARGUMENTS`.

## What to do

1. **Read the file** at `$ARGUMENTS`.

2. **Find the current `// ── Index:` line** — it appears before the first `use` statement. Note its current line number (call it `idx_line`).

3. **Scan the file for each key section** and record its actual current line number:

   | Section | What to look for |
   |---------|-----------------|
   | Helper functions | `^fn ` or `^pub fn ` before the struct definition |
   | Main struct | `^pub struct \w+Viz {` |
   | `new` | `pub fn new(` inside the struct impl block |
   | `impl Visualizer` | `^impl Visualizer for` |
   | `get_default_config` | `fn get_default_config(` |
   | `set_config` | `fn set_config(` |
   | `tick` | `fn tick(` |
   | `render` | `fn render(` |
   | `register` | `^pub fn register(` |
   | Any other notable named sections (e.g. `step_wave`, `ensure_grid`, `rms_to_color`) | `^fn \w+` before struct, or notable `^fn` inside impl |

4. **Build the new index string** using the format:
   ```
   // ── Index: Helper@N · StructName@N · new@N · impl@N · config@N · set_config@N · tick@N · render@N · register@N
   ```
   - Only include helper functions that are non-trivial and useful to jump to.
   - Use the struct's actual name (e.g. `TunnelViz`, not `Struct`).
   - Omit sections that don't exist in this file.

5. **Replace** the existing `// ── Index:` line with the new one. The line number of the index comment itself must not change — only its content changes.

6. **Confirm** by showing the old and new index lines side by side.

## Important

The index line numbers refer to line numbers **in the file as it will exist after the edit** (i.e., with the updated index comment in place). Since replacing the index comment content doesn't add or remove lines, the numbers you found in step 3 are correct as-is.

If the file has **no** `// ── Index:` comment yet, insert one immediately before the first `use` statement, then recalculate all section line numbers accounting for the +1 insertion offset.
