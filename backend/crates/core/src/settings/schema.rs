//! What settings exist: one [`Def`] per clap argument of every section.
//!
//! Nothing here is written by hand per setting. A section is a domain's
//! `#[derive(clap::Args)]` struct; its arguments become settings whose
//! `tokuly.toml` key is the environment variable without `TOKULY_` and the
//! section's own prefix (`TOKULY_MEDIA_MAX_VIDEO_HOURS` in `[media]` is
//! `max_video_hours`), and whose description, default and type come from
//! the argument's doc comment, default value and value parser.
use std::{any::TypeId, collections::HashMap, ffi::OsString};

use clap::{Arg, ArgAction, ArgMatches, Command};

/// The kinds of process a setting is read by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Process {
    Api,
    Realtime,
    Worker,
}

impl Process {
    pub fn name(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Realtime => "realtime",
            Self::Worker => "worker",
        }
    }
}

/// Validation a section runs over its whole configuration, given the parsed
/// arguments and whether the deployment is a production one (the domains'
/// `Config::validate`). Saving a console value runs it before the value is
/// stored.
pub type Check = fn(&ArgMatches, bool) -> anyhow::Result<()>;

/// A section of the settings: one domain's `clap::Args` struct and what the
/// domain says about its settings beyond what clap knows.
#[derive(Clone)]
pub struct SectionSpec {
    /// `[name]` in `tokuly.toml`, e.g. `media`.
    pub name: &'static str,
    /// `<Config as clap::Args>::augment_args`.
    pub augment: fn(Command) -> Command,
    /// What the section's variables start with after `TOKULY_` (and is left
    /// out of their keys); the upper-case name unless set.
    pub env_prefix: Option<&'static str>,
    /// Processes that read the section (and need a restart for a change of a setting that is not live).
    pub processes: &'static [Process],
    /// Keys read anew on every use: a console change applies at once.
    pub live: &'static [&'static str],
    /// Keys the staff console may change. Everything else is set in
    /// `tokuly.toml` or the environment only, so a new setting stays out of
    /// the console until its domain lists it (`<Config>::CONSOLE_EDITABLE`).
    pub editable: &'static [&'static str],
    /// Key prefixes that have a screen of their own in the console, with its path:
    /// the settings page links there instead of editing them.
    pub managed: &'static [(&'static str, &'static str)],
    pub check: Option<Check>,
}

impl SectionSpec {
    /// A domain section, read by the API and the workers, every setting
    /// editable and applied on restart.
    pub fn new(name: &'static str, augment: fn(Command) -> Command) -> Self {
        Self { name, augment, env_prefix: None, processes: &[Process::Api, Process::Worker], live: &[], editable: &[], managed: &[], check: None }
    }

    pub fn env_prefix(mut self, prefix: &'static str) -> Self {
        self.env_prefix = Some(prefix);
        self
    }

    pub fn processes(mut self, processes: &'static [Process]) -> Self {
        self.processes = processes;
        self
    }

    pub fn live(mut self, keys: &'static [&'static str]) -> Self {
        self.live = keys;
        self
    }

    pub fn editable(mut self, keys: &'static [&'static str]) -> Self {
        self.editable = keys;
        self
    }

    pub fn managed(mut self, prefixes: &'static [(&'static str, &'static str)]) -> Self {
        self.managed = prefixes;
        self
    }

    pub fn check(mut self, check: Check) -> Self {
        self.check = Some(check);
        self
    }
}

/// The type of a setting's value, as `tokuly.toml` writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Integer,
    /// Integer or decimal.
    Number,
    Text,
    /// Text values separated by the argument's delimiter (a TOML array).
    List,
}

/// One setting.
#[derive(Clone)]
pub struct Def {
    pub section: &'static str,
    /// The key inside the section.
    pub key: String,
    /// The clap argument ID (unique across sections).
    pub id: String,
    /// The environment variable, e.g. `TOKULY_MEDIA_MAX_VIDEO_HOURS`.
    pub env: String,
    /// The doc comment of the argument.
    pub help: String,
    /// The default as the command line would take it; `None` when unset by default.
    pub default: Option<String>,
    pub kind: Kind,
    pub delimiter: Option<char>,
    /// The values accepted, when the argument has a fixed list.
    pub choices: Vec<String>,
    /// Write-only: never shown, exported or logged.
    pub secret: bool,
    /// The console may change it ([`SectionSpec::editable`]).
    pub editable: bool,
    pub live: bool,
    /// The console path of the screen this setting is edited on instead.
    pub managed_at: Option<&'static str>,
    pub processes: &'static [Process],
    pub(crate) arg: Arg,
}

impl Def {
    /// `[section] key`, the way messages name a setting.
    pub fn name(&self) -> String {
        format!("[{}] {}", self.section, self.key)
    }

    /// Checks a value as the command line would take it and returns it in
    /// the form it is stored in, or what is wrong with it.
    pub fn check_value(&self, raw: &str) -> Result<String, String> {
        let long = self.arg.get_long().unwrap_or(self.id.as_str());
        let arg = self.arg.clone().env(None).required(false);
        let mut flag = OsString::from(format!("--{long}="));
        flag.push(raw);
        match Command::new("check").arg(arg).try_get_matches_from([OsString::from("check"), flag]) {
            Ok(_) => Ok(raw.to_owned()),
            Err(err) => Err(clap_message(&err)),
        }
    }
}

/// The doc comment of an argument. Clap drops the full stop of a one-line
/// summary; the console and the example file show sentences, so it is put back.
fn doc_text(arg: &Arg) -> String {
    let mut text = arg.get_long_help().or(arg.get_help()).map(|h| h.to_string()).unwrap_or_default().trim().to_owned();
    if text.chars().last().is_some_and(|c| c.is_alphanumeric() || matches!(c, ')' | '`' | '"')) {
        text.push('.');
    }
    text
}

/// The reason of a clap error without its usage text.
pub(crate) fn clap_message(err: &clap::Error) -> String {
    if let Some(source) = std::error::Error::source(err) {
        return source.to_string();
    }
    let text = err.to_string();
    let first = text.lines().next().unwrap_or_default();
    first.trim_start_matches("error: ").to_owned()
}

/// Every setting of every section.
pub struct Registry {
    sections: Vec<SectionSpec>,
    defs: Vec<Def>,
    by_name: HashMap<(String, String), usize>,
    by_id: HashMap<String, usize>,
}

impl Registry {
    /// Collects the settings of `sections`. Fails on what would make a
    /// setting unreachable: an argument without an environment variable or
    /// long flag, two settings with one key or ID, an action that takes no
    /// value, or a key named in a spec that does not exist.
    pub fn new(sections: Vec<SectionSpec>) -> anyhow::Result<Self> {
        let mut defs = Vec::new();
        let mut by_name = HashMap::new();
        let mut by_id = HashMap::new();
        for spec in &sections {
            anyhow::ensure!(
                !sections.iter().filter(|s| s.name == spec.name).nth(1).is_some(),
                "settings section [{}] is declared twice",
                spec.name
            );
            let cmd = (spec.augment)(Command::new(spec.name));
            let prefix = format!("{}_", spec.env_prefix.map(str::to_owned).unwrap_or_else(|| spec.name.to_ascii_uppercase()));
            for arg in cmd.get_arguments() {
                let id = arg.get_id().as_str().to_owned();
                let env = arg
                    .get_env()
                    .and_then(|e| e.to_str())
                    .ok_or_else(|| anyhow::anyhow!("setting `{id}` of [{}] has no environment variable", spec.name))?
                    .to_owned();
                anyhow::ensure!(arg.get_long().is_some(), "setting {env} has no long flag");
                let bare = env
                    .strip_prefix("TOKULY_")
                    .ok_or_else(|| anyhow::anyhow!("environment variable {env} does not start with TOKULY_"))?;
                let key = bare.strip_prefix(&prefix).unwrap_or(bare).to_ascii_lowercase();
                anyhow::ensure!(
                    matches!(arg.get_action(), ArgAction::Set | ArgAction::Append),
                    "setting {env} uses a clap action without a value; use `action = clap::ArgAction::Set`"
                );
                let delimiter = arg.get_value_delimiter();
                let parser = arg.get_value_parser();
                let kind = if delimiter.is_some() {
                    Kind::List
                } else if parser.type_id() == TypeId::of::<bool>() {
                    Kind::Bool
                } else if [
                    TypeId::of::<i8>(),
                    TypeId::of::<i16>(),
                    TypeId::of::<i32>(),
                    TypeId::of::<i64>(),
                    TypeId::of::<u8>(),
                    TypeId::of::<u16>(),
                    TypeId::of::<u32>(),
                    TypeId::of::<u64>(),
                    TypeId::of::<usize>(),
                ]
                .iter()
                .any(|t| parser.type_id() == *t)
                {
                    Kind::Integer
                } else if parser.type_id() == TypeId::of::<f64>() || parser.type_id() == TypeId::of::<f32>() {
                    Kind::Number
                } else {
                    Kind::Text
                };
                let choices =
                    if kind == Kind::Bool { Vec::new() } else { arg.get_possible_values().iter().map(|v| v.get_name().to_owned()).collect() };
                let default = (!arg.get_default_values().is_empty()).then(|| {
                    let values: Vec<String> = arg.get_default_values().iter().map(|v| v.to_string_lossy().into_owned()).collect();
                    values.join(&delimiter.unwrap_or(',').to_string())
                });
                let help = doc_text(arg);
                let managed_at = spec.managed.iter().find(|(p, _)| key.starts_with(p)).map(|(_, path)| *path);
                let def = Def {
                    section: spec.name,
                    live: spec.live.contains(&key.as_str()),
                    editable: spec.editable.contains(&key.as_str()),
                    secret: arg.is_hide_env_values_set(),
                    managed_at,
                    processes: spec.processes,
                    key,
                    id: id.clone(),
                    env,
                    help,
                    default,
                    kind,
                    delimiter,
                    choices,
                    arg: arg.clone(),
                };
                anyhow::ensure!(
                    by_name.insert((spec.name.to_owned(), def.key.clone()), defs.len()).is_none(),
                    "two settings are {}",
                    def.name()
                );
                anyhow::ensure!(by_id.insert(id.clone(), defs.len()).is_none(), "two settings have the argument ID `{id}`");
                defs.push(def);
            }
            for key in spec.live.iter().chain(spec.editable) {
                anyhow::ensure!(by_name.contains_key(&(spec.name.to_owned(), (*key).to_owned())), "[{}] names unknown setting `{key}`", spec.name);
            }
        }
        Ok(Self { sections, defs, by_name, by_id })
    }

    pub fn sections(&self) -> &[SectionSpec] {
        &self.sections
    }

    pub fn section(&self, name: &str) -> Option<&SectionSpec> {
        self.sections.iter().find(|s| s.name == name)
    }

    pub fn defs(&self) -> &[Def] {
        &self.defs
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&Def> {
        self.by_name.get(&(section.to_owned(), key.to_owned())).map(|&i| &self.defs[i])
    }

    pub fn by_id(&self, id: &str) -> Option<&Def> {
        self.by_id.get(id).map(|&i| &self.defs[i])
    }

    /// The keys of a section, for suggestions in error messages.
    pub(crate) fn keys_of<'a>(&'a self, section: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.defs.iter().filter(move |d| d.section == section).map(|d| d.key.as_str())
    }
}

#[cfg(test)]
mod tests {
    use clap::Args;

    use super::*;

    #[derive(Args)]
    struct Demo {
        /// How many.
        #[arg(long, env = "TOKULY_DEMO_LIMIT", default_value_t = 5)]
        limit: i64,
        #[arg(long, env = "TOKULY_DEMO_ON", default_value_t = false, action = clap::ArgAction::Set)]
        on: bool,
        #[arg(long, env = "TOKULY_DEMO_HOSTS", value_delimiter = ',', default_value = "a,b")]
        hosts: Vec<String>,
        #[arg(long, env = "TOKULY_DEMO_TOKEN", hide_env_values = true)]
        token: Option<String>,
        #[arg(long, env = "TOKULY_OTHER_NAME", default_value = "x", value_parser = ["x", "y"])]
        other_name: String,
    }

    #[test]
    fn keys_types_and_defaults_come_from_clap() {
        let registry = Registry::new(vec![SectionSpec::new("demo", Demo::augment_args).live(&["limit"])]).unwrap();
        let limit = registry.get("demo", "limit").unwrap();
        assert_eq!((limit.kind, limit.default.as_deref(), limit.help.as_str(), limit.live), (Kind::Integer, Some("5"), "How many.", true));
        assert_eq!(registry.get("demo", "on").unwrap().kind, Kind::Bool);
        let hosts = registry.get("demo", "hosts").unwrap();
        assert_eq!((hosts.kind, hosts.default.as_deref()), (Kind::List, Some("a,b")));
        let token = registry.get("demo", "token").unwrap();
        assert!(token.secret && token.default.is_none());
        // Without the section's prefix the variable keeps its whole name.
        let other = registry.get("demo", "other_name").unwrap();
        assert_eq!(other.choices, ["x", "y"]);
        assert_eq!(other.check_value("z").unwrap_err().contains("'z'"), true);
        assert_eq!(limit.check_value("7"), Ok("7".to_owned()));
        assert!(limit.check_value("seven").is_err());
    }

    #[test]
    fn unknown_keys_in_a_spec_are_refused() {
        let err = Registry::new(vec![SectionSpec::new("demo", Demo::augment_args).live(&["limt"])]).err().unwrap();
        assert!(err.to_string().contains("limt"));
    }
}
