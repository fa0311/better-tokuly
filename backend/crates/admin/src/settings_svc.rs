//! `ServerSettingsService`: 「サーバー設定」, the console layer of
//! `tokuly_core::settings`. Every RPC needs the `admin` role.
//!
//! A value is saved only where no higher layer (the environment, the
//! command line, `tokuly.toml`) sets the setting, never for bootstrap
//! settings, and never for settings another screen of the console edits.
//! Secrets are write-only: what is returned about them is whether one is
//! set and the last four characters of one saved here.
use std::collections::HashMap;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{
    events as core_events,
    prelude::*,
    settings::{self, Def, Kind, Process, Source, State, store},
};
use tokuly_proto::admin::*;

use crate::{
    Admin,
    audit::Audit,
    events::{SERVER_SETTING_CHANGED, ServerSettingChanged},
    staff::Staff,
};

/// Longest value accepted (a list of origins, a PEM key).
const VALUE_MAX_CHARS: usize = 16 * 1024;
const UPDATES_PER_MINUTE: u32 = 60;

fn source_pb(source: Source) -> ServerSettingSource {
    match source {
        Source::Default => ServerSettingSource::SERVER_SETTING_SOURCE_DEFAULT,
        Source::Console => ServerSettingSource::SERVER_SETTING_SOURCE_CONSOLE,
        Source::File => ServerSettingSource::SERVER_SETTING_SOURCE_FILE,
        Source::Env => ServerSettingSource::SERVER_SETTING_SOURCE_ENVIRONMENT,
        Source::CommandLine => ServerSettingSource::SERVER_SETTING_SOURCE_COMMAND_LINE,
    }
}

fn kind_pb(kind: Kind) -> ServerSettingKind {
    match kind {
        Kind::Bool => ServerSettingKind::SERVER_SETTING_KIND_BOOL,
        Kind::Integer => ServerSettingKind::SERVER_SETTING_KIND_INTEGER,
        Kind::Number => ServerSettingKind::SERVER_SETTING_KIND_NUMBER,
        Kind::Text => ServerSettingKind::SERVER_SETTING_KIND_TEXT,
        Kind::List => ServerSettingKind::SERVER_SETTING_KIND_LIST,
    }
}

fn process_pb(process: Process) -> ServerProcess {
    match process {
        Process::Api => ServerProcess::SERVER_PROCESS_API,
        Process::Realtime => ServerProcess::SERVER_PROCESS_REALTIME,
        Process::Worker => ServerProcess::SERVER_PROCESS_WORKER,
    }
}

/// Why the console may not change `def` while `source` is in force.
fn lock_of(def: &Def, source: Source) -> ServerSettingLock {
    if def.managed_at.is_some() {
        ServerSettingLock::SERVER_SETTING_LOCK_MANAGED_ELSEWHERE
    } else if def.bootstrap {
        ServerSettingLock::SERVER_SETTING_LOCK_BOOTSTRAP
    } else if source.locks() {
        ServerSettingLock::SERVER_SETTING_LOCK_OVERRIDDEN
    } else {
        ServerSettingLock::SERVER_SETTING_LOCK_NONE
    }
}

/// One setting as the console shows it; secrets without any value.
fn setting_pb(state: &State, def: &Def, secrets: &HashMap<String, store::SecretInfo>) -> ServerSetting {
    let (value, source) = state.layers.effective(def);
    let console = state.layers.console.get(&def.id).or_else(|| state.rejected.get(&def.id));
    let shown = |v: Option<&str>| if def.secret { None } else { v.map(str::to_owned) };
    ServerSetting {
        section: def.section.to_owned(),
        key: def.key.clone(),
        env: def.env.clone(),
        description: def.help.clone(),
        kind: kind_pb(def.kind).into(),
        choices: def.choices.clone(),
        default_value: shown(def.default.as_deref()),
        value: shown(value),
        source: source_pb(source).into(),
        secret: def.secret,
        secret_set: def.secret && value.is_some_and(|v| !v.is_empty()),
        secret_last4: if def.secret && source == Source::Console { secrets.get(&def.id).map(|s| s.last4.clone()).unwrap_or_default() } else { String::new() },
        lock: lock_of(def, source).into(),
        managed_at: def.managed_at.unwrap_or_default().to_owned(),
        live: def.live,
        processes: def.processes.iter().map(|p| process_pb(*p).into()).collect(),
        restart_pending: state.restart_pending(def),
        console_value: shown(console.map(String::as_str)),
        console_secret_set: def.secret && console.is_some(),
        console_rejected: state.rejection(def).unwrap_or_default().to_owned(),
        ..Default::default()
    }
}

impl Admin {
    fn settings_state(&self) -> AppResult<std::sync::Arc<State>> {
        self.core.settings.state().ok_or_else(|| AppError::unavailable("SETTINGS_UNAVAILABLE", "this process has no settings registry"))
    }

    fn setting_def<'a>(state: &'a State, section: &str, key: &str) -> AppResult<&'a Def> {
        state.registry.get(section.trim(), key.trim()).or_not_found("SETTING_NOT_FOUND", "no such setting")
    }
}

impl ServerSettingsService for Admin {
    async fn list_server_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListServerSettingsRequest>,
    ) -> ServiceResult<ListServerSettingsResponse> {
        handle(async {
            Staff::admin(&ctx)?;
            let state = self.settings_state()?;
            let secrets = store::secrets(&self.core.db, &state.registry).await?;
            ok(ListServerSettingsResponse {
                settings: state.registry.defs().iter().map(|d| setting_pb(&state, d, &secrets)).collect(),
                file_path: state.file.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_server_setting(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateServerSettingRequest>,
    ) -> ServiceResult<UpdateServerSettingResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            self.core.cache.rate_limit(&format!("admin:settings:{}", staff.id), UPDATES_PER_MINUTE, std::time::Duration::from_secs(60)).await?;
            let state = self.settings_state()?;
            let def = Self::setting_def(&state, request.section, request.key)?;
            if let Some(path) = def.managed_at {
                bail!(precondition, "SETTING_MANAGED_ELSEWHERE", "{} is edited at {path}", def.name());
            }
            ensure!(!def.bootstrap, precondition, "SETTING_BOOTSTRAP", "bootstrap settings are set in the environment or tokuly.toml only");
            let (_, source) = state.layers.effective(def);
            if source.locks() {
                bail!(precondition, "SETTING_LOCKED", "{} is set by {}, which wins over the console", def.name(), source.label());
            }
            let value = if request.reset {
                None
            } else {
                ensure!(request.value.chars().count() <= VALUE_MAX_CHARS, invalid, "SETTING_TOO_LONG", "values are at most {VALUE_MAX_CHARS} characters");
                Some(def.check_value(request.value).map_err(|e| AppError::invalid("SETTING_INVALID", e))?)
            };
            // The domain's own rules, over the section as it would be: with
            // every console value it has, set-aside ones included.
            let mut layers = state.layers.clone();
            layers.console = state.console_of_section(def.section);
            match &value {
                Some(value) => layers.console.insert(def.id.clone(), value.clone()),
                None => layers.console.remove(&def.id),
            };
            settings::check_section(&state.registry, &layers, def.section, state.production)
                .map_err(|e| AppError::invalid("SETTING_INVALID", e.to_string()))?;

            let mut tx = self.core.db.begin().await?;
            let change = store::write(&mut tx, &self.core.keys, def, value.as_deref(), staff.id).await?;
            if let Some(change) = &change {
                Audit::by(&staff, "server_setting.updated")
                    .setting(def.section, &def.key)
                    .before(serde_json::json!({ "value": change.old }))
                    .after(serde_json::json!({ "value": change.new }))
                    .record(&mut *tx)
                    .await?;
                let payload = ServerSettingChanged { section: def.section.to_owned(), key: def.key.clone(), secret: def.secret, live: def.live };
                core_events::emit(&mut *tx, SERVER_SETTING_CHANGED, Some(staff.id), &payload).await?;
            }
            tx.commit().await?;
            if change.is_some() {
                self.core.settings_changed().await.map_err(AppError::internal)?;
            }
            let state = self.settings_state()?;
            let def = Self::setting_def(&state, def.section, &def.key)?;
            let secrets = store::secrets(&self.core.db, &state.registry).await?;
            ok(UpdateServerSettingResponse { setting: setting_pb(&state, def, &secrets).into(), ..Default::default() })
        })
        .await
    }

    async fn export_server_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ExportServerSettingsRequest>,
    ) -> ServiceResult<ExportServerSettingsResponse> {
        handle(async {
            Staff::admin(&ctx)?;
            let state = self.settings_state()?;
            let values = state
                .registry
                .defs()
                .iter()
                .filter(|d| !d.secret)
                .filter_map(|d| match state.layers.effective(d) {
                    (Some(value), source) if source != Source::Default => {
                        let note = match source {
                            Source::Env => format!("from {}", d.env),
                            other => format!("from {}", other.label()),
                        };
                        Some((d.id.clone(), (value.to_owned(), note)))
                    }
                    _ => None,
                })
                .collect();
            let header = format!(
                "tokuly.toml exported from the staff console (サーバー設定) on {}.\n\
                 The settings in force that are not defaults. Secrets are left out: keep them in .env.\n\
                 Every key is described in tokuly.example.toml.",
                now().format("%Y-%m-%d")
            );
            ok(ExportServerSettingsResponse { toml: settings::file::export(&state.registry, &values, &header), ..Default::default() })
        })
        .await
    }

    async fn list_server_setting_changes(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListServerSettingChangesRequest>,
    ) -> ServiceResult<ListServerSettingChangesResponse> {
        handle(async {
            Staff::admin(&ctx)?;
            let state = self.settings_state()?;
            let setting = match (request.section.trim(), request.key.trim()) {
                ("", "") => None,
                (section, key) => Some(Self::setting_def(&state, section, key)?),
            };
            let page = Page::from_view(request.page.as_option(), 100);
            let mut rows =
                store::changes(&self.core.db, setting.map(|d| (d.section, d.key.as_str())), page.decode::<Uuid>()?, page.fetch()).await?;
            let info = page.finish(&mut rows, |r| r.id);
            let actors: Vec<Uuid> = rows.iter().filter_map(|r| r.actor_id).collect();
            let people = tokuly_identity::summaries(&self.core, &actors).await?;
            let changes = rows
                .into_iter()
                .map(|r| ServerSettingChange {
                    id: r.id.to_string(),
                    actor: r.actor_id.and_then(|id| people.get(&id).cloned()).into(),
                    section: r.section,
                    key: r.key,
                    old_value: r.old_value,
                    new_value: r.new_value,
                    secret: r.secret,
                    changed_at: ts(r.created_at),
                    ..Default::default()
                })
                .collect();
            ok(ListServerSettingChangesResponse { changes, page: info.into(), ..Default::default() })
        })
        .await
    }
}
