/// config.rs — Config persistence and JSON merge/validation helpers.
///
/// Ported unchanged in behaviour from the terminal app: one JSON file per
/// visualizer under the platform config directory, merged against the
/// visualizer's default schema on load.

use crate::visualizer::Visualizer;

/// Return the platform-correct config file path for the named visualizer.
///
/// macOS:       ~/Library/Application Support/audio_viz/{name}.json
/// Linux/other: $XDG_CONFIG_HOME/audio_viz/{name}.json
///              (falls back to ~/.config/audio_viz/{name}.json)
pub fn config_path(name: &str) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("audio_viz")
            .join(format!("{name}.json"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                std::path::PathBuf::from(home).join(".config")
            });
        base.join("audio_viz").join(format!("{name}.json"))
    }
}

/// Load saved config for the active visualizer, apply it, and write back the
/// merged/cleaned version.  Silently ignores I/O or parse errors so a corrupt
/// file never prevents startup.
pub fn load_and_apply_config(viz: &mut Box<dyn Visualizer>) {
    let path = config_path(viz.name());
    let saved = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return,
    };
    if let Ok(cleaned) = viz.set_config(&saved) {
        // Write back the cleaned version to drop obsolete keys and fill
        // in any new fields added since the config was last saved.
        let _ = write_config(viz.name(), &cleaned);
    }
}

/// The visualizer's current effective config: the saved file merged over the
/// schema defaults (or plain defaults when nothing is saved).  Used to seed
/// the settings UI without mutating the visualizer.
pub fn live_config(viz: &dyn Visualizer) -> String {
    let default = viz.get_default_config();
    match std::fs::read_to_string(config_path(viz.name())) {
        Ok(saved) if !saved.is_empty() => merge_config(&default, &saved),
        _ => default,
    }
}

/// Persist the current config to disk.
pub fn write_config(name: &str, json: &str) -> std::io::Result<()> {
    let path = config_path(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, json)
}

/// Merge a partial config JSON string into the default config.
///
/// The merge operates on the `config` array, matching entries by `"name"`.
///
///   - Entries in `default` not in `partial`  → kept with default value
///   - Entries in `partial` not in `default`  → silently dropped
///   - Entries in both                         → partial value applied if it
///       passes type / range / variants check; otherwise default is kept
///
/// Returns the complete merged JSON string (pretty-printed).
/// Returns `default` unchanged on any parse failure.
pub fn merge_config(default: &str, partial: &str) -> String {
    let default_val: serde_json::Value = match serde_json::from_str(default) {
        Ok(v) => v,
        Err(_) => return default.to_string(),
    };
    let partial_val: serde_json::Value = match serde_json::from_str(partial) {
        Ok(v) => v,
        Err(_) => return default.to_string(),
    };

    let default_config = match default_val["config"].as_array() {
        Some(arr) => arr.clone(),
        None => return default.to_string(),
    };

    let empty_arr: Vec<serde_json::Value> = Vec::new();
    let partial_config = partial_val["config"].as_array().unwrap_or(&empty_arr);

    // Build a name → value map from the partial config
    let partial_values: std::collections::HashMap<&str, &serde_json::Value> = partial_config
        .iter()
        .filter_map(|entry| {
            let name = entry["name"].as_str()?;
            let value = entry.get("value")?;
            Some((name, value))
        })
        .collect();

    // Merge: for each schema entry apply the partial value when it validates
    let merged: Vec<serde_json::Value> = default_config
        .iter()
        .map(|def| {
            let name = match def["name"].as_str() {
                Some(n) => n,
                None => return def.clone(),
            };
            let Some(&partial_val) = partial_values.get(name) else {
                return def.clone();
            };
            let kind = def["type"].as_str().unwrap_or("");
            if validate_config_value(def, kind, partial_val) {
                let mut merged_entry = def.clone();
                merged_entry["value"] = partial_val.clone();
                merged_entry
            } else {
                def.clone()
            }
        })
        .collect();

    let mut result = default_val.clone();
    result["config"] = serde_json::Value::Array(merged);
    serde_json::to_string_pretty(&result).unwrap_or_else(|_| default.to_string())
}

/// Validate a candidate config value against its schema entry.
fn validate_config_value(
    schema: &serde_json::Value,
    kind: &str,
    value: &serde_json::Value,
) -> bool {
    match kind {
        "float" => {
            let Some(v) = value.as_f64() else { return false };
            if let Some(min) = schema["min"].as_f64() {
                if v < min {
                    return false;
                }
            }
            if let Some(max) = schema["max"].as_f64() {
                if v > max {
                    return false;
                }
            }
            true
        }
        "int" => {
            let Some(v) = value.as_i64().or_else(|| value.as_f64().map(|f| f as i64)) else {
                return false;
            };
            if let Some(min) = schema["min"].as_i64() {
                if v < min {
                    return false;
                }
            }
            if let Some(max) = schema["max"].as_i64() {
                if v > max {
                    return false;
                }
            }
            true
        }
        "enum" => {
            let Some(v_str) = value.as_str() else { return false };
            let Some(variants) = schema["variants"].as_array() else { return false };
            variants.iter().any(|var| var.as_str() == Some(v_str))
        }
        "bool" => value.as_bool().is_some(),
        _ => false,
    }
}
