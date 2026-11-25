//! Server settings: every `clap` setting of the binary, from four layers.
//!
//! Highest first:
//!
//! 1. the command line and the `TOKULY_*` environment variables (containers, CI),
//! 2. `tokuly.toml` (the file named by `TOKULY_CONFIG`), the declarative source,
//! 3. the staff console (「サーバー設定」), stored in `server_settings`,
//! 4. the default of the argument.
//!
//! The layers become clap defaults ([`apply`]), so clap itself still parses
//! and validates every value and the environment keeps working exactly as
//! before. A domain declares a setting by adding an argument to its
//! `clap::Args` struct; [`SectionSpec`] says which of them are applied at
//! once ([`Settings::current`]) and which the console must not touch.
pub mod file;
mod schema;
pub mod store;

use std::{
    any::{Any, TypeId},
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::OsString,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use clap::{ArgMatches, Command, parser::ValueSource};
use parking_lot::{Mutex, RwLock};
pub use schema::{Check, Def, Kind, Process, Registry, SectionSpec};

/// Where a setting's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    Console,
    File,
    Env,
    CommandLine,
}

impl Source {
    /// Values from these layers cannot be changed from the console.
    pub fn locks(self) -> bool {
        matches!(self, Self::File | Self::Env | Self::CommandLine)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Console => "管理画面 (console)",
            Self::File => "tokuly.toml",
            Self::Env => "環境変数 (environment)",
            Self::CommandLine => "command line",
        }
    }
}

/// The values of each layer, by argument ID, as the command line takes them.
#[derive(Debug, Clone, Default)]
pub struct Layers {
    pub cli: HashMap<String, String>,
    pub env: HashMap<String, String>,
    pub file: HashMap<String, String>,
    pub console: HashMap<String, String>,
    /// Settings whose environment variable is set but empty, and for which
    /// empty is not a value ([`Def::empty_is_value`]): treated as unset (and
    /// clap is told not to read them), since an env file line `TOKULY_X=`
    /// means "nothing here" there.
    pub empty_env: BTreeSet<String>,
}

impl Layers {
    /// The value in force and its layer.
    pub fn effective<'a>(&'a self, def: &'a Def) -> (Option<&'a str>, Source) {
        let id = &def.id;
        if let Some(v) = self.cli.get(id) {
            (Some(v), Source::CommandLine)
        } else if let Some(v) = self.env.get(id) {
            (Some(v), Source::Env)
        } else if let Some(v) = self.file.get(id) {
            (Some(v), Source::File)
        } else if let Some(v) = self.console.get(id) {
            (Some(v), Source::Console)
        } else {
            (def.default.as_deref(), Source::Default)
        }
    }

    /// The environment layer: every setting whose variable is set.
    pub fn read_env(&mut self, registry: &Registry, lookup: impl Fn(&str) -> Option<String>) {
        self.env.clear();
        self.empty_env.clear();
        for def in registry.defs() {
            match lookup(&def.env) {
                Some(value) if value.is_empty() && !def.empty_is_value => {
                    self.empty_env.insert(def.id.clone());
                }
                Some(value) => {
                    self.env.insert(def.id.clone(), value);
                }
                None => {}
            }
        }
    }

    /// What to tell the operator about empty variables that are ignored
    /// while another value is in force (a line that once meant "empty").
    pub fn empty_env_warnings(&self, registry: &Registry) -> Vec<String> {
        self.empty_env
            .iter()
            .filter_map(|id| registry.by_id(id))
            .filter_map(|def| match self.effective(def) {
                (Some(value), source) if !value.is_empty() => Some(format!(
                    "{} is empty and ignored; {} = {} from {} applies (delete the line, or set the value in tokuly.toml)",
                    def.env,
                    def.name(),
                    if def.secret { "(secret)" } else { value },
                    source.label()
                )),
                _ => None,
            })
            .collect()
    }

    /// The command-line layer from a first, lenient parse of the arguments.
    pub fn read_cli(&mut self, registry: &Registry, matches: &ArgMatches) {
        self.cli = registry
            .defs()
            .iter()
            .filter(|d| matches.try_contains_id(&d.id).unwrap_or(false) && matches.value_source(&d.id) == Some(ValueSource::CommandLine))
            .filter_map(|d| {
                let raw: Vec<String> = matches.get_raw(&d.id)?.map(|v| v.to_string_lossy().into_owned()).collect();
                Some((d.id.clone(), raw.join(&d.delimiter.unwrap_or(',').to_string())))
            })
            .collect();
    }
}

/// Makes the lower layers clap defaults of `cmd`'s arguments. The
/// environment stays clap's own (it reads the same variables); the command
/// line becomes a default too when `with_cli` (re-parsing a section without
/// the original arguments).
pub fn apply(cmd: Command, registry: &Registry, layers: &Layers, with_cli: bool) -> Command {
    cmd.mut_args(|arg| {
        let Some(def) = registry.by_id(arg.get_id().as_str()) else { return arg };
        let arg = if layers.empty_env.contains(&def.id) { arg.env(None) } else { arg };
        let value = with_cli
            .then(|| layers.cli.get(&def.id))
            .flatten()
            .or_else(|| layers.file.get(&def.id))
            .or_else(|| layers.console.get(&def.id));
        match value {
            Some(value) => {
                let arg = arg.default_value(value.clone()).required(false);
                if def.secret { arg.hide_default_value(true) } else { arg }
            }
            None => arg,
        }
    })
}

/// Re-parses one section from `layers` and runs its check: what saving a
/// console value would lead to, before it is saved.
pub fn check_section(registry: &Registry, layers: &Layers, section: &str, production: bool) -> anyhow::Result<()> {
    let spec = registry.section(section).ok_or_else(|| anyhow::anyhow!("unknown section [{section}]"))?;
    let cmd = apply((spec.augment)(Command::new(spec.name).no_binary_name(true)), registry, layers, true);
    let matches = cmd.try_get_matches_from(Vec::<OsString>::new()).map_err(|e| anyhow::anyhow!(schema::clap_message(&e)))?;
    match spec.check {
        Some(check) => check(&matches, production),
        None => Ok(()),
    }
}

/// A console layer after [`admit_console`].
#[derive(Debug, Default)]
pub struct Admitted {
    pub kept: HashMap<String, String>,
    pub rejected: HashMap<String, String>,
    /// Why each refused section was refused.
    pub reasons: BTreeMap<String, String>,
}

/// Checks the console values of each section together with the layers
/// above them ([`check_section`]) and sets aside every console value of a
/// section that fails: a mistake saved in the console must never stop the
/// server. Each refusal is logged.
pub fn admit_console(registry: &Registry, base: &Layers, console: HashMap<String, String>, production: bool) -> Admitted {
    let mut by_section: BTreeMap<&str, HashMap<String, String>> = BTreeMap::new();
    for (id, value) in console {
        if let Some(def) = registry.by_id(&id) {
            by_section.entry(def.section).or_default().insert(id, value);
        }
    }
    let mut admitted = Admitted::default();
    for (section, values) in by_section {
        let candidate = Layers { console: values.clone(), ..base.clone() };
        match check_section(registry, &candidate, section, production) {
            Ok(()) => admitted.kept.extend(values),
            Err(e) => {
                tracing::warn!(section, error = %e, "settings: the console values of this section are set aside (they do not pass its checks)");
                admitted.reasons.insert(section.to_owned(), e.to_string());
                admitted.rejected.extend(values);
            }
        }
    }
    admitted
}

/// The settings a process runs with.
pub struct State {
    pub registry: Arc<Registry>,
    pub layers: Layers,
    /// The values in force when the process started, by argument ID.
    pub started: HashMap<String, Option<String>>,
    /// The `tokuly.toml` that was read, if any.
    pub file: Option<PathBuf>,
    pub production: bool,
    /// Console values set aside because their section did not pass its
    /// checks with them, by argument ID (see [`admit_console`]).
    pub rejected: HashMap<String, String>,
    /// Why, by section.
    pub rejected_sections: BTreeMap<String, String>,
    /// Counts the states a process has had: what [`Settings::current`]
    /// caches its parses by (an address would be reused once freed).
    generation: u64,
}

impl State {
    pub fn new(registry: Arc<Registry>, layers: Layers, file: Option<PathBuf>, production: bool) -> Self {
        let started = registry.defs().iter().map(|d| (d.id.clone(), layers.effective(d).0.map(str::to_owned))).collect();
        Self { registry, layers, started, file, production, rejected: HashMap::new(), rejected_sections: BTreeMap::new(), generation: 0 }
    }

    /// Like [`State::new`], with a console layer that is first checked
    /// section by section ([`admit_console`]): what the process starts with.
    pub fn with_console(registry: Arc<Registry>, mut layers: Layers, console: HashMap<String, String>, file: Option<PathBuf>, production: bool) -> Self {
        layers.console = HashMap::new();
        let admitted = admit_console(&registry, &layers, console, production);
        layers.console = admitted.kept;
        let mut state = Self::new(registry, layers, file, production);
        state.rejected = admitted.rejected;
        state.rejected_sections = admitted.reasons;
        state
    }

    /// Why the console value of `def` is set aside, if it is.
    pub fn rejection(&self, def: &Def) -> Option<&str> {
        self.rejected.contains_key(&def.id).then(|| self.rejected_sections.get(def.section).map(String::as_str)).flatten()
    }

    /// The console values of a section, set aside ones included: what a change is checked together with.
    pub fn console_of_section(&self, section: &str) -> HashMap<String, String> {
        let mut values = self.layers.console.clone();
        values.extend(self.rejected.iter().filter(|(id, _)| self.registry.by_id(id).is_some_and(|d| d.section == section)).map(|(k, v)| (k.clone(), v.clone())));
        values
    }

    /// Whether this process runs with another value than the one in force
    /// now (a console change of a setting that is not live).
    pub fn restart_pending(&self, def: &Def) -> bool {
        !def.live && self.started.get(&def.id).map(Option::as_deref) != Some(self.layers.effective(def).0)
    }
}

/// The process's settings, shared by every domain through [`crate::Core`].
/// Empty (nothing registered) in tests that build domains by hand.
#[derive(Clone, Default)]
pub struct Settings(Arc<SettingsInner>);

#[derive(Default)]
struct SettingsInner {
    state: RwLock<Option<Arc<State>>>,
    /// The generation of the next state installed.
    next_generation: AtomicU64,
    /// Sections that did not parse (each logged once).
    failed_parses: AtomicU64,
    /// Sections re-parsed by [`Settings::current`], with the generation of
    /// the state they were parsed from; `None` when they did not parse.
    parsed: Mutex<HashMap<TypeId, (u64, Option<Arc<dyn Any + Send + Sync>>)>>,
}

impl Settings {
    pub fn new(state: State) -> Self {
        let settings = Self::default();
        settings.install(state);
        settings
    }

    pub fn install(&self, mut state: State) {
        let mut guard = self.0.state.write();
        state.generation = self.0.next_generation.fetch_add(1, Ordering::Relaxed);
        *guard = Some(Arc::new(state));
    }

    pub fn state(&self) -> Option<Arc<State>> {
        self.0.state.read().clone()
    }

    /// The console values as last loaded, set-aside ones included.
    pub fn console(&self) -> Option<HashMap<String, String>> {
        let state = self.state()?;
        let mut values = state.layers.console.clone();
        values.extend(state.rejected.clone());
        Some(values)
    }

    /// Replaces the console layer (after a change was saved here or in
    /// another process), setting aside the values of sections that do not
    /// pass their checks with them.
    pub fn set_console(&self, console: HashMap<String, String>) {
        let mut guard = self.0.state.write();
        if let Some(state) = guard.as_ref() {
            let base = Layers { console: HashMap::new(), ..state.layers.clone() };
            let admitted = admit_console(&state.registry, &base, console, state.production);
            let next = State {
                registry: state.registry.clone(),
                layers: Layers { console: admitted.kept, ..base },
                started: state.started.clone(),
                file: state.file.clone(),
                production: state.production,
                rejected: admitted.rejected,
                rejected_sections: admitted.reasons,
                generation: self.0.next_generation.fetch_add(1, Ordering::Relaxed),
            };
            *guard = Some(Arc::new(next));
        }
    }

    /// A section as it is in force now, console changes included: what a
    /// domain reads for its live settings. `None` when no settings are
    /// registered (tests), or when the section does not parse (it was
    /// checked on saving; the start-up value then stays in force).
    pub fn current<C>(&self, section: &str) -> Option<Arc<C>>
    where
        C: clap::Args + clap::FromArgMatches + Send + Sync + 'static,
    {
        let state = self.state()?;
        let stamp = state.generation;
        if let Some((parsed_from, value)) = self.0.parsed.lock().get(&TypeId::of::<C>())
            && *parsed_from == stamp
        {
            return value.clone().and_then(|v| v.downcast::<C>().ok());
        }
        let cmd = apply(C::augment_args(Command::new(section.to_owned()).no_binary_name(true)), &state.registry, &state.layers, true);
        let parsed = match cmd.try_get_matches_from(Vec::<OsString>::new()).map_err(|e| schema::clap_message(&e)).and_then(|m| C::from_arg_matches(&m).map_err(|e| schema::clap_message(&e))) {
            Ok(parsed) => Arc::new(parsed),
            Err(e) => {
                self.0.failed_parses.fetch_add(1, Ordering::Relaxed);
                tracing::error!(section, error = %e, "settings: section does not parse with the console values");
                self.0.parsed.lock().insert(TypeId::of::<C>(), (stamp, None));
                return None;
            }
        };
        self.0.parsed.lock().insert(TypeId::of::<C>(), (stamp, Some(parsed.clone())));
        Some(parsed)
    }
}

#[cfg(test)]
mod tests {
    use clap::{Args, FromArgMatches, Parser};

    use super::*;

    #[derive(Args, Debug)]
    struct Demo {
        #[arg(long, env = "TOKULY_DEMOSET_LIMIT", default_value_t = 5)]
        limit: i64,
        #[arg(long, env = "TOKULY_DEMOSET_HOSTS", value_delimiter = ',', default_value = "a,b")]
        hosts: Vec<String>,
        #[arg(long, env = "TOKULY_DEMOSET_TOKEN", hide_env_values = true)]
        token: String,
    }

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        demo: Demo,
    }

    fn registry() -> Arc<Registry> {
        Arc::new(Registry::new(vec![SectionSpec::new("demoset", Demo::augment_args).live(&["limit"])]).unwrap())
    }

    fn def<'a>(r: &'a Registry, key: &str) -> &'a Def {
        r.get("demoset", key).unwrap()
    }

    #[test]
    fn each_layer_wins_over_the_ones_below() {
        let r = registry();
        let limit = def(&r, "limit");
        let mut layers = Layers::default();
        assert_eq!(layers.effective(limit), (Some("5"), Source::Default));
        layers.console.insert("limit".into(), "6".into());
        assert_eq!(layers.effective(limit), (Some("6"), Source::Console));
        layers.file.insert("limit".into(), "7".into());
        assert_eq!(layers.effective(limit), (Some("7"), Source::File));
        layers.env.insert("limit".into(), "8".into());
        assert_eq!(layers.effective(limit), (Some("8"), Source::Env));
        layers.cli.insert("limit".into(), "9".into());
        assert_eq!(layers.effective(limit), (Some("9"), Source::CommandLine));
    }

    /// `TOKULY_X=` in an env file means "not set", not "set to empty".
    #[test]
    fn empty_environment_variables_are_unset() {
        let r = registry();
        let mut layers = Layers::default();
        layers.read_env(&r, |name| (name == "TOKULY_DEMOSET_LIMIT").then(String::new));
        layers.file.insert("limit".into(), "7".into());
        assert_eq!(layers.effective(def(&r, "limit")), (Some("7"), Source::File));
    }

    #[test]
    fn layers_become_what_clap_parses() {
        let r = registry();
        let mut layers = Layers::default();
        layers.file.insert("hosts".into(), "x,y,z".into());
        layers.console.insert("limit".into(), "6".into());
        layers.console.insert("token".into(), "from-console".into());
        let cmd = apply(<Cli as clap::CommandFactory>::command(), &r, &layers, false);
        let help = cmd.clone().render_long_help().to_string();
        assert!(!help.contains("from-console"), "a secret default must not show in --help");
        let matches = cmd.try_get_matches_from(["t"]).unwrap();
        let cli = Cli::from_arg_matches(&matches).unwrap();
        assert_eq!((cli.demo.limit, cli.demo.hosts, cli.demo.token.as_str()), (6, vec!["x".to_owned(), "y".into(), "z".into()], "from-console"));
        // The command line still wins over every layer.
        let matches = apply(<Cli as clap::CommandFactory>::command(), &r, &layers, false).try_get_matches_from(["t", "--limit", "2"]).unwrap();
        assert_eq!(Cli::from_arg_matches(&matches).unwrap().demo.limit, 2);
    }

    #[test]
    fn current_follows_the_console_layer() {
        let r = registry();
        let mut layers = Layers::default();
        layers.cli.insert("token".into(), "t".into());
        let settings = Settings::new(State::new(r.clone(), layers, None, false));
        assert_eq!(settings.current::<Demo>("demoset").unwrap().limit, 5);
        settings.set_console(HashMap::from([("limit".to_owned(), "11".to_owned())]));
        assert_eq!(settings.current::<Demo>("demoset").unwrap().limit, 11);
        let state = settings.state().unwrap();
        // `limit` is live: no restart pending; the token would need one.
        assert!(!state.restart_pending(def(&r, "limit")));
        settings.set_console(HashMap::from([("token".to_owned(), "u".to_owned())]));
        let state = settings.state().unwrap();
        // The command line wins over the console, so nothing changed for the token.
        assert!(!state.restart_pending(def(&r, "token")));
        assert!(Settings::default().current::<Demo>("demoset").is_none());
    }

    /// A section that does not parse is logged once per state, not on
    /// every read (domains read live settings per request).
    #[test]
    fn a_parse_failure_is_reported_once() {
        // No token anywhere: the section lacks a required value.
        let settings = Settings::new(State::new(registry(), Layers::default(), None, false));
        for _ in 0..3 {
            assert!(settings.current::<Demo>("demoset").is_none());
        }
        assert_eq!(settings.0.failed_parses.load(Ordering::Relaxed), 1);
    }

    /// Every change is seen, also when the allocator hands out a freed
    /// state's address again (the parse cache once keyed on it).
    #[test]
    fn current_sees_every_change_in_a_row() {
        let mut layers = Layers::default();
        layers.cli.insert("token".into(), "t".into());
        let settings = Settings::new(State::new(registry(), layers, None, false));
        let set = |limit: i64| settings.set_console(HashMap::from([("limit".to_owned(), limit.to_string())]));
        for limit in (5..200).step_by(2) {
            set(limit);
            assert_eq!(settings.current::<Demo>("demoset").unwrap().limit, limit);
            // Two changes without a read in between: the second state may
            // sit where the first one was.
            set(limit + 100);
            set(limit + 1);
            assert_eq!(settings.current::<Demo>("demoset").unwrap().limit, limit + 1);
        }
    }
}
