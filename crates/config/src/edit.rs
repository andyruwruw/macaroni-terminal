//! Changing one setting in the config file, as the settings dialog does: the rest of the file,
//! comments included, stays as the user left it. The watcher then reloads it like any edit.
//!
//! [`write_setting`] is the entry point; it edits the document with `toml_edit` rather than
//! re-serializing a [`Config`], which would drop comments and the template's commented defaults.

use std::path::Path;

use crate::settings::{Config, SshProfile, TEMPLATE};

/// A value to write with [`write_setting`]. Each variant becomes the TOML value of the same
/// kind.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `true` or `false`.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A float.
    Float(f64),
    /// A quoted string.
    Str(String),
    /// An array of strings, e.g. `shell.args`.
    List(Vec<String>),
}

/// Sets `key` (dotted, e.g. `font.size`) in `<dir>/config.toml`, or removes it (back to the
/// default) when `value` is `None`. A missing file starts from the template.
///
/// Writes the file (creating `dir` if needed); the config watcher then picks the change up like
/// any edit. Fails with a readable message if the current file isn't valid TOML, if a table on
/// the key's path is some other kind of value, or if the write fails.
pub fn write_setting(dir: &Path, key: &str, value: Option<Value>) -> Result<(), String> {
    let path = Config::path(dir);
    let source = std::fs::read_to_string(&path).unwrap_or_else(|_| TEMPLATE.to_owned());
    let updated = set(&source, key, value)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(&path, updated).map_err(|e| format!("{}: {e}", path.display()))
}

/// [`write_setting`] on a document in memory: returns `source` with `key` set or removed.
/// Missing tables on the way are created; a new key goes at the end of its table.
fn set(source: &str, key: &str, value: Option<Value>) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = source.parse().map_err(|e| format!("{e}"))?;
    let (tables, name) = match key.rsplit_once('.') {
        Some((tables, name)) => (tables.split('.').collect(), name),
        None => (Vec::new(), key),
    };
    let mut table = doc.as_table_mut();
    for t in tables {
        let entry = table
            .entry(t)
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
        table = entry
            .as_table_mut()
            .ok_or_else(|| format!("{t} isn't a table"))?;
    }
    match value {
        Some(value) => {
            let value: toml_edit::Value = match value {
                Value::Bool(v) => v.into(),
                Value::Int(v) => v.into(),
                Value::Float(v) => v.into(),
                Value::Str(v) => v.into(),
                Value::List(v) => v
                    .iter()
                    .map(String::as_str)
                    .collect::<toml_edit::Array>()
                    .into(),
            };
            table[name] = toml_edit::value(value);
        }
        None => {
            table.remove(name);
        }
    }
    Ok(doc.to_string())
}

/// Replaces the `[[ssh]]` profiles in `<dir>/config.toml` with `profiles`, keeping the rest of
/// the file as it is. Unset fields aren't written. A missing file starts from the template.
pub fn write_ssh(dir: &Path, profiles: &[SshProfile]) -> Result<(), String> {
    let path = Config::path(dir);
    let source = std::fs::read_to_string(&path).unwrap_or_else(|_| TEMPLATE.to_owned());
    let updated = set_ssh(&source, profiles)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(&path, updated).map_err(|e| format!("{}: {e}", path.display()))
}

/// [`write_ssh`] on a document in memory.
fn set_ssh(source: &str, profiles: &[SshProfile]) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = source.parse().map_err(|e| format!("{e}"))?;
    if profiles.is_empty() {
        doc.remove("ssh");
        return Ok(doc.to_string());
    }
    let mut tables = toml_edit::ArrayOfTables::new();
    for profile in profiles {
        let mut table = toml_edit::Table::new();
        if !profile.name.trim().is_empty() {
            table["name"] = toml_edit::value(&profile.name);
        }
        table["host"] = toml_edit::value(&profile.host);
        if let Some(user) = &profile.user {
            table["user"] = toml_edit::value(user);
        }
        if let Some(port) = profile.port {
            table["port"] = toml_edit::value(i64::from(port));
        }
        if let Some(key) = &profile.identity_file {
            table["identity_file"] = toml_edit::value(key);
        }
        if !profile.args.is_empty() {
            let args: toml_edit::Array = profile.args.iter().map(String::as_str).collect();
            table["args"] = toml_edit::value(args);
        }
        tables.push(table);
    }
    doc["ssh"] = toml_edit::Item::ArrayOfTables(tables);
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_colors_apply() {
        let source = set(TEMPLATE, "ui_theme", Some(Value::Str("solid".into()))).unwrap();
        let source = set(
            &source,
            "colors.solid.tab_active",
            Some(Value::Str("#123456".into())),
        )
        .unwrap();
        let config = Config::parse(&source, Path::new(".")).unwrap();
        assert_eq!(
            config.colors.solid.get("tab_active"),
            Some(&crate::ColorValue::Rgba(
                crate::Rgba::parse("#123456").unwrap()
            ))
        );
    }

    #[test]
    fn ssh_profiles_round_trip() {
        let profiles = vec![
            SshProfile {
                name: "Pi".into(),
                host: "pi@raspberrypi.local".into(),
                port: Some(2222),
                ..SshProfile::default()
            },
            SshProfile {
                host: "work".into(),
                args: vec!["-A".into()],
                ..SshProfile::default()
            },
        ];
        let source = set_ssh(TEMPLATE, &profiles).unwrap();
        assert!(source.contains("# Color theme"));
        let config = Config::parse(&source, Path::new(".")).unwrap();
        assert_eq!(config.ssh, profiles);
        assert_eq!(
            config.ssh[0].ssh_args(),
            ["-p", "2222", "pi@raspberrypi.local"]
        );
        assert_eq!(config.ssh[1].label(), "work");

        let source = set_ssh(&source, &profiles[1..]).unwrap();
        assert_eq!(
            Config::parse(&source, Path::new(".")).unwrap().ssh,
            profiles[1..]
        );
        let source = set_ssh(&source, &[]).unwrap();
        assert!(
            Config::parse(&source, Path::new("."))
                .unwrap()
                .ssh
                .is_empty()
        );
    }

    #[test]
    fn keeps_comments_and_round_trips() {
        let source = set(TEMPLATE, "font.size", Some(Value::Int(16))).unwrap();
        let source = set(&source, "ui_theme", Some(Value::Str("glass".into()))).unwrap();
        let source = set(&source, "notifications.bell", Some(Value::Bool(true))).unwrap();
        assert!(source.contains("# Color theme"));
        let config = Config::parse(&source, Path::new(".")).unwrap();
        assert_eq!(config.font.size, 16.0);
        assert_eq!(config.ui_theme, crate::UiTheme::Glass);
        assert!(config.notifications.bell);

        let source = set(&source, "font.size", None).unwrap();
        let config = Config::parse(&source, Path::new(".")).unwrap();
        assert_eq!(config.font.size, Config::default().font.size);
    }
}
