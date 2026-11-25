//! `tokuly.toml`: reading it strictly, and writing the example file and the
//! console's export.
use std::collections::HashMap;

use super::schema::{Def, Kind, Registry};

/// Reads a `tokuly.toml` into values by argument ID. Anything that is not a
/// known setting with a value of its type is an error naming the key, so
/// a typo never goes unnoticed.
pub fn parse(registry: &Registry, text: &str, origin: &str) -> anyhow::Result<HashMap<String, String>> {
    let table: toml::Table = toml::from_str(text).map_err(|e| anyhow::anyhow!("{origin}: {e}"))?;
    let mut values = HashMap::new();
    for (section, entries) in table {
        let Some(entries) = entries.as_table() else {
            anyhow::bail!("{origin}: `{section}` must be a section such as [core] or [media], with the setting inside it");
        };
        if registry.section(&section).is_none() {
            let known: Vec<&str> = registry.sections().iter().map(|s| s.name).collect();
            anyhow::bail!("{origin}: unknown section [{section}]{}", suggestion(&section, known.iter().copied()));
        }
        for (key, value) in entries {
            let Some(def) = registry.get(&section, key) else {
                anyhow::bail!("{origin}: unknown setting [{section}] {key}{}", suggestion(key, registry.keys_of(&section)));
            };
            let raw = from_toml(def, value).map_err(|e| anyhow::anyhow!("{origin}: {}: {e}", def.name()))?;
            let raw = def.check_value(&raw).map_err(|e| anyhow::anyhow!("{origin}: {}: {e}", def.name()))?;
            values.insert(def.id.clone(), raw);
        }
    }
    Ok(values)
}

/// A TOML value as the command line would take it, if it has the setting's type.
fn from_toml(def: &Def, value: &toml::Value) -> Result<String, String> {
    use toml::Value as V;
    match (def.kind, value) {
        (Kind::Bool, V::Boolean(b)) => Ok(b.to_string()),
        (Kind::Integer | Kind::Number, V::Integer(i)) => Ok(i.to_string()),
        (Kind::Number, V::Float(f)) => Ok(f.to_string()),
        (Kind::Text, V::String(s)) => Ok(s.clone()),
        (Kind::List, V::Array(items)) => {
            let delimiter = def.delimiter.unwrap_or(',');
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                let text = match item {
                    V::String(s) => s.clone(),
                    V::Integer(i) => i.to_string(),
                    other => return Err(format!("expected a list of strings, found {} in it", other.type_str())),
                };
                if text.contains(delimiter) {
                    return Err(format!("the list item `{text}` contains `{delimiter}`; give each item separately"));
                }
                parts.push(text);
            }
            Ok(parts.join(&delimiter.to_string()))
        }
        (kind, other) => Err(format!("expected {}, found {}", kind_text(kind), other.type_str())),
    }
}

fn kind_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Bool => "true or false",
        Kind::Integer => "an integer",
        Kind::Number => "a number",
        Kind::Text => "a string",
        Kind::List => "a list of strings, e.g. [\"a\", \"b\"]",
    }
}

/// ` (did you mean `x`?)` for the closest known name, if one is close.
fn suggestion<'a>(given: &str, known: impl Iterator<Item = &'a str>) -> String {
    known
        .map(|k| (distance(given, k), k))
        .filter(|(d, k)| *d <= 2.max(k.len() / 4))
        .min()
        .map(|(_, k)| format!(" (did you mean `{k}`?)"))
        .unwrap_or_default()
}

/// Levenshtein distance.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let current = row[j + 1];
            row[j + 1] = if ca == *cb { previous } else { 1 + previous.min(row[j]).min(row[j + 1]) };
            previous = current;
        }
    }
    row[b.len()]
}

/// A stored value written as TOML.
pub fn to_toml(def: &Def, raw: &str) -> String {
    match def.kind {
        Kind::Bool | Kind::Integer | Kind::Number => raw.to_owned(),
        Kind::Text => toml::Value::String(raw.to_owned()).to_string(),
        Kind::List => {
            if raw.is_empty() {
                return "[]".to_owned();
            }
            let items: Vec<String> =
                raw.split(def.delimiter.unwrap_or(',')).map(|s| toml::Value::String(s.to_owned()).to_string()).collect();
            format!("[{}]", items.join(", "))
        }
    }
}

/// Writes `text` as `# ` comment lines of at most about 100 columns.
fn comment(out: &mut String, text: &str) {
    const WIDTH: usize = 98;
    for line in text.lines() {
        if line.trim().is_empty() {
            out.push_str("#\n");
            continue;
        }
        let mut current = String::new();
        for word in line.split_whitespace() {
            if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > WIDTH {
                out.push_str(&format!("# {current}\n"));
                current.clear();
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        out.push_str(&format!("# {current}\n"));
    }
}

const EXAMPLE_HEADER: &str = "\
# tokuly.example.toml — every setting of the Tokuly backend, with its default.
#
# Generated by `tokuly config example`; do not edit (CI checks it is current).
# Copy what you want to change into config/tokuly.toml (Compose mounts that
# directory at /etc/tokuly; elsewhere point TOKULY_CONFIG at the file) and
# uncomment it. Unknown keys and values of the wrong type stop the server
# with a message naming the key.
#
# Where a value comes from, highest first:
#   1. the environment variable named on each setting (for containers and CI)
#   2. this file
#   3. the staff console (サーバー設定), which cannot change what 1 or 2 set
#   4. the default shown here
#
# Secrets (the master secret, passwords, API keys) are better kept in .env;
# only settings marked `console` can be changed from the console.
";

/// The example file: every setting, commented out at its default.
pub fn example(registry: &Registry) -> String {
    let mut out = String::from(EXAMPLE_HEADER);
    for spec in registry.sections() {
        out.push_str(&format!("\n[{}]\n", spec.name));
        for def in registry.defs().iter().filter(|d| d.section == spec.name) {
            out.push('\n');
            comment(&mut out, &def.help);
            let mut notes = vec![def.env.clone()];
            if def.secret {
                notes.push("secret".to_owned());
            }
            if def.editable {
                notes.push("console".to_owned());
            }
            if def.empty_is_value {
                notes.push("empty is a value".to_owned());
            }
            if def.live {
                notes.push("applies at once".to_owned());
            } else {
                notes.push(format!("restart: {}", def.processes.iter().map(|p| p.name()).collect::<Vec<_>>().join(", ")));
            }
            if !def.choices.is_empty() {
                notes.push(format!("one of: {}", def.choices.join(", ")));
            }
            out.push_str(&format!("# ({})\n", notes.join("; ")));
            match &def.default {
                Some(value) => out.push_str(&format!("# {} = {}\n", def.key, to_toml(def, value))),
                None => out.push_str(&format!("# {} = {}  # not set by default\n", def.key, empty(def))),
            }
        }
    }
    out
}

fn empty(def: &Def) -> &'static str {
    match def.kind {
        Kind::List => "[]",
        Kind::Bool => "false",
        Kind::Integer | Kind::Number => "0",
        Kind::Text => "\"\"",
    }
}

/// The given values (by argument ID, with a note on where each came from)
/// as a `tokuly.toml`, grouped by section in registry order.
pub fn export(registry: &Registry, values: &HashMap<String, (String, String)>, header: &str) -> String {
    let mut out = String::new();
    comment(&mut out, header);
    for spec in registry.sections() {
        let defs: Vec<&Def> = registry.defs().iter().filter(|d| d.section == spec.name && values.contains_key(&d.id)).collect();
        if defs.is_empty() {
            continue;
        }
        out.push_str(&format!("\n[{}]\n", spec.name));
        for def in defs {
            let (value, note) = &values[&def.id];
            out.push_str(&format!("{} = {}  # {note}\n", def.key, to_toml(def, value)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use clap::Args;

    use super::*;
    use crate::settings::SectionSpec;

    #[derive(Args)]
    struct Demo {
        #[arg(long, env = "TOKULY_DEMO_LIMIT", default_value_t = 5)]
        limit: i64,
        #[arg(long, env = "TOKULY_DEMO_ON", default_value_t = false, action = clap::ArgAction::Set)]
        on: bool,
        #[arg(long, env = "TOKULY_DEMO_HOSTS", value_delimiter = ',', default_value = "a,b")]
        hosts: Vec<String>,
        #[arg(long, env = "TOKULY_DEMO_NAME", default_value = "x")]
        name: String,
    }

    fn registry() -> Registry {
        Registry::new(vec![SectionSpec::new("demo", Demo::augment_args)]).unwrap()
    }

    #[test]
    fn values_of_the_right_type_are_read() {
        let values = parse(&registry(), "[demo]\nlimit = 7\non = true\nhosts = [\"h1\", \"h2\"]\nname = \"n\"\n", "t.toml").unwrap();
        assert_eq!(values["limit"], "7");
        assert_eq!(values["on"], "true");
        assert_eq!(values["hosts"], "h1,h2");
        assert_eq!(values["name"], "n");
    }

    #[test]
    fn unknown_keys_and_wrong_types_name_the_key() {
        let r = registry();
        let err = parse(&r, "[demo]\nlimt = 7\n", "t.toml").unwrap_err().to_string();
        assert!(err.contains("unknown setting [demo] limt") && err.contains("did you mean `limit`"), "{err}");
        let err = parse(&r, "[demoo]\nlimit = 7\n", "t.toml").unwrap_err().to_string();
        assert!(err.contains("unknown section [demoo]"), "{err}");
        let err = parse(&r, "[demo]\nlimit = \"seven\"\n", "t.toml").unwrap_err().to_string();
        assert!(err.contains("[demo] limit: expected an integer, found string"), "{err}");
        let err = parse(&r, "[demo]\non = 1\n", "t.toml").unwrap_err().to_string();
        assert!(err.contains("[demo] on: expected true or false"), "{err}");
        let err = parse(&r, "[demo]\nhosts = \"a\"\n", "t.toml").unwrap_err().to_string();
        assert!(err.contains("[demo] hosts: expected a list"), "{err}");
        let err = parse(&r, "limit = 7\n", "t.toml").unwrap_err().to_string();
        assert!(err.contains("`limit` must be a section"), "{err}");
        // Out of range for the argument's type: the value parser says why.
        let err = parse(&r, "[demo]\nlimit = 99999999999999999999\n", "t.toml").unwrap_err().to_string();
        assert!(err.contains("t.toml"), "{err}");
    }

    #[test]
    fn the_example_round_trips_when_uncommented() {
        let r = registry();
        let example = example(&r);
        let uncommented: String = example
            .lines()
            .filter(|l| l.starts_with('[') || (l.starts_with("# ") && l.contains(" = ")))
            .map(|l| l.trim_start_matches("# ").to_owned() + "\n")
            .collect();
        let values = parse(&r, &uncommented, "example").unwrap();
        assert_eq!(values["limit"], "5");
        assert_eq!(values["hosts"], "a,b");
    }
}
