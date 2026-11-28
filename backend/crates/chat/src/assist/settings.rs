//! Who may use which AI provider: the operator's settings (the environment
//! as initial values, then what the staff console saved), each user's
//! choice and own API keys, and each user's consent to having their
//! messages sent by other people's requests.
//!
//! API keys are sealed at rest ([`tokuly_core::keys::Keys::seal`]) and only
//! ever leave this module towards the provider they belong to: what clients
//! see is whether one is set and its last four characters. Nothing here
//! derives `Debug`, so no key can end up in a log line by accident.
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tokuly_core::{
    egress::{self, HostAllowance, Policy},
    prelude::*,
};
use tokuly_identity::age;
use tokuly_proto::chat::{self as pb, AiProvider};
use url::Url;

use crate::{Chat, ChatConfig};

/// Seal labels of operator keys and users' keys.
const INSTANCE_KEY_SEAL: &str = "chat/ai-key/instance";
const USER_KEY_SEAL: &str = "chat/ai-key/user";

pub(crate) const SEALED: [tokuly_core::rekey::Sealed; 2] = [
    tokuly_core::rekey::Sealed::bytes("chat_ai_providers", "api_key_sealed", INSTANCE_KEY_SEAL),
    tokuly_core::rekey::Sealed::bytes("chat_ai_user_keys", "key_sealed", USER_KEY_SEAL),
];
/// The order in which providers are listed and the default is picked.
pub(crate) const PROVIDERS: [AiProvider; 3] =
    [AiProvider::AI_PROVIDER_ANTHROPIC, AiProvider::AI_PROVIDER_OPENAI, AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE];
const MAX_BASE_URL_CHARS: usize = 500;
const MAX_API_KEY_CHARS: usize = 512;
const MIN_API_KEY_CHARS: usize = 8;
const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
/// The hosts of the companies' own APIs.
const ANTHROPIC_HOST: &str = "api.anthropic.com";
const OPENAI_HOST: &str = "api.openai.com";

pub(crate) fn provider_number(provider: AiProvider) -> i16 {
    provider as i16
}

pub(crate) fn provider_from(number: i16) -> Option<AiProvider> {
    PROVIDERS.into_iter().find(|provider| provider_number(*provider) == number)
}

/// Whether users may bring a key of their own for `provider`.
pub(crate) fn accepts_own_key(provider: AiProvider) -> bool {
    matches!(provider, AiProvider::AI_PROVIDER_ANTHROPIC | AiProvider::AI_PROVIDER_OPENAI)
}

/// One provider as the operator configured it. No `Debug`: it holds a key.
#[derive(Clone)]
pub(crate) struct ProviderSettings {
    pub provider: AiProvider,
    pub enabled: bool,
    pub model: String,
    pub base_url: String,
    /// The opened operator key.
    pub api_key: Option<String>,
    pub allow_private_host: bool,
    /// No row was saved: these are the environment's values.
    pub from_environment: bool,
    pub updated_at: Option<DateTime<Utc>>,
}

impl ProviderSettings {
    /// Who receives the text: the company, or the compatible server's host.
    /// The company's name only when the base URL is the company's own
    /// API: a base URL pointing elsewhere (a proxy such as OpenRouter, a
    /// gateway) sends the text there, so that host is named instead.
    pub fn recipient(&self) -> String {
        let host = Url::parse(&self.base_url).ok().and_then(|url| url.host_str().map(str::to_ascii_lowercase));
        match (self.provider, host.as_deref()) {
            (AiProvider::AI_PROVIDER_ANTHROPIC, Some(ANTHROPIC_HOST)) => "Anthropic".to_owned(),
            (AiProvider::AI_PROVIDER_OPENAI, Some(OPENAI_HOST)) => "OpenAI".to_owned(),
            (_, Some(host)) => host.to_owned(),
            (_, None) => "?".to_owned(),
        }
    }

    /// Enabled, with a model and a base URL.
    pub fn configured(&self) -> bool {
        self.enabled && !self.model.is_empty() && !self.base_url.is_empty()
    }

    /// Can serve a user who brought `own_key` (or none).
    pub fn usable_with(&self, own_key: bool) -> bool {
        self.configured()
            && match self.provider {
                AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE => true,
                _ => self.api_key.is_some() || own_key,
            }
    }

    /// The outbound guard's exception for this provider's host, when the
    /// operator granted it.
    pub fn allowance(&self) -> Option<HostAllowance> {
        if !self.allow_private_host || self.provider != AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE {
            return None;
        }
        Url::parse(&self.base_url).ok().as_ref().and_then(HostAllowance::for_url)
    }

    fn last4(&self) -> String {
        self.api_key.as_deref().map(last4).unwrap_or_default()
    }

    pub fn to_config(&self) -> pb::AssistProviderConfig {
        pb::AssistProviderConfig {
            provider: self.provider.into(),
            enabled: self.enabled,
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            api_key_set: self.api_key.is_some(),
            api_key_last4: self.last4(),
            allow_private_host: self.allow_private_host,
            from_environment: self.from_environment,
            updated_at: opt_ts(self.updated_at),
            ready: self.usable_with(false),
            ..Default::default()
        }
    }
}

/// The last four characters of a key, to recognize it by.
fn last4(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

/// The operator's settings of every provider.
pub(crate) struct Instance {
    pub providers: Vec<ProviderSettings>,
    /// As saved (or from the environment); `None`: the first enabled one.
    pub default: Option<AiProvider>,
}

impl Instance {
    pub fn get(&self, provider: AiProvider) -> Option<&ProviderSettings> {
        self.providers.iter().find(|settings| settings.provider == provider)
    }

    /// The provider users without a choice get: the default when it is
    /// enabled, else the first enabled one.
    pub fn effective_default(&self) -> Option<AiProvider> {
        self.default
            .filter(|provider| self.get(*provider).is_some_and(ProviderSettings::configured))
            .or_else(|| self.providers.iter().find(|settings| settings.configured()).map(|settings| settings.provider))
    }

    pub fn any_enabled(&self) -> bool {
        self.providers.iter().any(ProviderSettings::configured)
    }

    pub fn to_proto(&self) -> pb::AssistInstance {
        pb::AssistInstance {
            providers: self.providers.iter().map(ProviderSettings::to_config).collect(),
            default_provider: self.default.unwrap_or(AiProvider::AI_PROVIDER_UNSPECIFIED).into(),
            effective_default: self.effective_default().unwrap_or(AiProvider::AI_PROVIDER_UNSPECIFIED).into(),
            ..Default::default()
        }
    }
}

/// What the environment says about a provider.
fn from_environment(cfg: &ChatConfig, provider: AiProvider) -> ProviderSettings {
    let key = |value: &str| (!value.is_empty()).then(|| value.to_owned());
    let (enabled, model, base_url, api_key, allow_private_host) = match provider {
        AiProvider::AI_PROVIDER_ANTHROPIC => (
            !cfg.anthropic_api_key.is_empty(),
            cfg.ai_model.clone(),
            cfg.anthropic_base_url.clone(),
            key(&cfg.anthropic_api_key),
            false,
        ),
        AiProvider::AI_PROVIDER_OPENAI => (
            !cfg.openai_api_key.is_empty(),
            cfg.openai_model.clone(),
            cfg.openai_base_url.clone(),
            key(&cfg.openai_api_key),
            false,
        ),
        _ => (
            !cfg.compat_base_url.is_empty() && !cfg.compat_model.is_empty(),
            cfg.compat_model.clone(),
            cfg.compat_base_url.clone(),
            key(&cfg.compat_api_key),
            cfg.compat_allow_private_host,
        ),
    };
    ProviderSettings {
        provider,
        enabled,
        model,
        base_url: base_url.trim_end_matches('/').to_owned(),
        api_key,
        allow_private_host,
        from_environment: true,
        updated_at: None,
    }
}

#[derive(sqlx::FromRow)]
struct ProviderRow {
    provider: i16,
    enabled: bool,
    model: String,
    base_url: String,
    api_key_sealed: Option<Vec<u8>>,
    allow_private_host: bool,
    updated_at: DateTime<Utc>,
}

/// One user's assistant settings.
pub(crate) struct UserSettings {
    /// `None`: the instance default.
    pub provider: Option<AiProvider>,
    pub off: bool,
    /// `None`: not stored yet (see [`Chat::ai_message_use_allowed`]).
    pub allow_message_use: Option<bool>,
    /// The owner chose the consent setting themselves.
    pub message_use_chosen: bool,
    /// 「Tokuly Bot と話す」; `None`: not stored yet (see [`Chat::bot_enabled_for`]).
    pub bot_enabled: Option<bool>,
    /// The owner chose `bot_enabled` themselves.
    pub bot_chosen: bool,
}

/// A key a user brought, opened.
pub(crate) struct OwnKey {
    pub provider: AiProvider,
    pub key: String,
    pub created_at: DateTime<Utc>,
}

/// The provider a user's requests go to, with the key to use.
pub(crate) struct Route {
    pub settings: ProviderSettings,
    /// The user's own key, or the operator's.
    pub api_key: Option<String>,
}

/// Why a user has no provider.
pub(crate) enum NoRoute {
    /// The operator enabled nothing (usable).
    Disabled,
    /// The user switched the assistant off.
    TurnedOff,
    /// The user's choice is not enabled or not usable.
    ChoiceUnavailable,
}

/// Picks the provider for a user: their choice when they made one (only
/// that one: their text never goes to a company they did not pick), else
/// the instance default.
pub(crate) fn route(instance: &Instance, user: &UserSettings, own_keys: &[OwnKey]) -> Result<Route, NoRoute> {
    if user.off {
        return Err(NoRoute::TurnedOff);
    }
    let (provider, explicit) = match user.provider {
        Some(provider) => (provider, true),
        None => (instance.effective_default().ok_or(NoRoute::Disabled)?, false),
    };
    let own = own_keys.iter().find(|key| key.provider == provider && accepts_own_key(provider));
    match instance.get(provider) {
        Some(settings) if settings.usable_with(own.is_some()) => Ok(Route {
            settings: settings.clone(),
            api_key: own.map(|own| own.key.clone()).or_else(|| settings.api_key.clone()),
        }),
        _ if explicit => Err(NoRoute::ChoiceUnavailable),
        _ => Err(NoRoute::Disabled),
    }
}

/// Validated operator input for one provider.
pub(crate) struct ProviderUpdate {
    pub provider: AiProvider,
    pub enabled: bool,
    pub model: String,
    pub base_url: String,
    pub key: KeyChange,
    pub allow_private_host: bool,
}

pub(crate) enum KeyChange {
    Keep,
    Set(String),
    Clear,
}

impl KeyChange {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Keep => "kept",
            Self::Set(_) => "set",
            Self::Clear => "cleared",
        }
    }
}

/// An API key as typed: printable ASCII without spaces, of a sane length.
pub(crate) fn check_api_key(key: &str) -> AppResult<String> {
    let key = key.trim();
    ensure!(
        (MIN_API_KEY_CHARS..=MAX_API_KEY_CHARS).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic()),
        invalid,
        "API_KEY_INVALID",
        "an API key is {MIN_API_KEY_CHARS}-{MAX_API_KEY_CHARS} printable characters without spaces"
    );
    Ok(key.to_owned())
}

/// A known provider, or PROVIDER_INVALID.
pub(crate) fn known_provider(value: &buffa::EnumValue<AiProvider>) -> AppResult<AiProvider> {
    value
        .as_known()
        .filter(|provider| *provider != AiProvider::AI_PROVIDER_UNSPECIFIED)
        .ok_or_else(|| AppError::invalid("PROVIDER_INVALID", "choose an AI provider"))
}

/// Checks an operator's base URL: http(s), nothing but scheme, host, port
/// and path, and an address the outbound guard (with the allowance, when
/// asked for) lets through. Returns it without a trailing slash.
pub(crate) fn check_base_url(
    provider: AiProvider,
    base_url: &str,
    allow_private_host: bool,
    policy: Policy,
) -> AppResult<String> {
    let base_url = base_url.trim();
    let base_url = match (base_url.is_empty(), provider) {
        (true, AiProvider::AI_PROVIDER_ANTHROPIC) => ANTHROPIC_BASE_URL,
        (true, AiProvider::AI_PROVIDER_OPENAI) => OPENAI_BASE_URL,
        _ => base_url,
    };
    let invalid = || {
        AppError::invalid(
            "BASE_URL_INVALID",
            "the base URL must be an http(s) URL without credentials, query or fragment",
        )
    };
    ensure!(base_url.chars().count() <= MAX_BASE_URL_CHARS, invalid, "BASE_URL_INVALID", "the base URL is too long");
    let url = Url::parse(base_url).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let allowance = if allow_private_host { HostAllowance::for_url(&url) } else { None };
    if let Err(refusal) = egress::check_url_allowing(&url, policy, allowance.as_ref()) {
        bail!(precondition, "BASE_URL_NOT_ALLOWED", "the outbound guard refuses this address: {refusal}");
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

impl Chat {
    /// The outbound guard's policy for AI requests.
    pub(crate) fn ai_policy(&self) -> Policy {
        Policy { allow_loopback: self.cfg.ai_allow_loopback }
    }

    /// The operator's settings: saved rows over the environment's values.
    pub(crate) async fn ai_instance(&self) -> AppResult<Instance> {
        let rows: Vec<ProviderRow> = sqlx::query_as(
            "SELECT provider, enabled, model, base_url, api_key_sealed, allow_private_host, updated_at FROM chat_ai_providers",
        )
        .fetch_all(&self.core.db)
        .await?;
        let saved_default: Option<i16> =
            sqlx::query_scalar("SELECT default_provider FROM chat_ai_instance").fetch_optional(&self.core.db).await?;
        let mut rows: HashMap<i16, ProviderRow> = rows.into_iter().map(|row| (row.provider, row)).collect();
        let providers = PROVIDERS
            .into_iter()
            .map(|provider| match rows.remove(&provider_number(provider)) {
                None => from_environment(&self.cfg, provider),
                Some(row) => ProviderSettings {
                    provider,
                    enabled: row.enabled,
                    model: row.model,
                    base_url: row.base_url,
                    api_key: row.api_key_sealed.as_deref().and_then(|sealed| {
                        let opened =
                            self.core.keys.open(INSTANCE_KEY_SEAL, sealed).and_then(|key| String::from_utf8(key).ok());
                        if opened.is_none() {
                            tracing::error!(
                                ?provider,
                                "a saved AI API key could not be opened (was TOKULY_SECRET changed?)"
                            );
                        }
                        opened
                    }),
                    allow_private_host: row.allow_private_host,
                    from_environment: false,
                    updated_at: Some(row.updated_at),
                },
            })
            .collect();
        let default = match saved_default {
            Some(number) => provider_from(number),
            None => self.cfg.default_provider().ok().flatten(),
        };
        Ok(Instance { providers, default })
    }

    /// Saves one provider's operator settings and records who did it.
    pub(crate) async fn save_ai_provider(&self, admin_id: Uuid, update: ProviderUpdate) -> AppResult<()> {
        let environment = from_environment(&self.cfg, update.provider);
        let keep = matches!(update.key, KeyChange::Keep);
        // The key a new row starts with: the new one, none, or (kept) the
        // environment's, so saving other fields never loses it.
        let initial_key = match &update.key {
            KeyChange::Set(key) => Some(key.clone()),
            KeyChange::Clear => None,
            KeyChange::Keep => environment.api_key.clone(),
        };
        let sealed = initial_key.as_ref().map(|key| self.core.keys.seal(INSTANCE_KEY_SEAL, key.as_bytes()));
        let last4 = initial_key.as_deref().map(last4).unwrap_or_default();
        let private_host = if update.allow_private_host {
            Url::parse(&update.base_url).ok().and_then(|url| url.host_str().map(str::to_owned))
        } else {
            None
        };
        let mut tx = self.core.db.begin().await?;
        sqlx::query(
            "INSERT INTO chat_ai_providers (provider, enabled, model, base_url, api_key_sealed, api_key_last4, allow_private_host, updated_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (provider) DO UPDATE SET
                enabled = EXCLUDED.enabled, model = EXCLUDED.model, base_url = EXCLUDED.base_url,
                api_key_sealed = CASE WHEN $9 THEN chat_ai_providers.api_key_sealed ELSE EXCLUDED.api_key_sealed END,
                api_key_last4 = CASE WHEN $9 THEN chat_ai_providers.api_key_last4 ELSE EXCLUDED.api_key_last4 END,
                allow_private_host = EXCLUDED.allow_private_host, updated_by = EXCLUDED.updated_by, updated_at = now()",
        )
        .bind(provider_number(update.provider))
        .bind(update.enabled)
        .bind(&update.model)
        .bind(&update.base_url)
        .bind(sealed)
        .bind(last4)
        .bind(update.allow_private_host)
        .bind(admin_id)
        .bind(keep)
        .execute(&mut *tx)
        .await?;
        let event = crate::events::AssistProviderUpdated {
            provider: update.provider as i32,
            enabled: update.enabled,
            model: update.model.clone(),
            base_url: update.base_url.clone(),
            api_key: update.key.name().to_owned(),
            allow_private_host: update.allow_private_host,
            private_host: private_host.clone(),
        };
        tokuly_core::events::emit(&mut *tx, crate::events::ASSIST_PROVIDER_UPDATED, Some(admin_id), &event).await?;
        tx.commit().await?;
        tracing::info!(provider = ?update.provider, enabled = update.enabled, model = %update.model, base_url = %update.base_url, api_key = update.key.name(), %admin_id, "AI provider settings saved");
        if let Some(host) = private_host {
            tracing::warn!(%host, %admin_id, "AI provider host allowed past the outbound guard's private-address rule");
        }
        Ok(())
    }

    pub(crate) async fn save_ai_default(&self, admin_id: Uuid, provider: Option<AiProvider>) -> AppResult<()> {
        let number = provider.map(provider_number).unwrap_or(0);
        let mut tx = self.core.db.begin().await?;
        sqlx::query(
            "INSERT INTO chat_ai_instance (singleton, default_provider, updated_by) VALUES (true, $1, $2)
             ON CONFLICT (singleton) DO UPDATE SET default_provider = EXCLUDED.default_provider,
                updated_by = EXCLUDED.updated_by, updated_at = now()",
        )
        .bind(number)
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
        let event = crate::events::AssistDefaultChanged { provider: i32::from(number) };
        tokuly_core::events::emit(&mut *tx, crate::events::ASSIST_DEFAULT_CHANGED, Some(admin_id), &event).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn ai_user_settings(&self, user_id: Uuid) -> AppResult<UserSettings> {
        let row: Option<(i16, bool, Option<bool>, bool, Option<bool>, bool)> = sqlx::query_as(
            "SELECT provider, off, allow_message_use, message_use_chosen, bot_enabled, bot_chosen
             FROM chat_ai_user_settings WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?;
        Ok(match row {
            Some((provider, off, allow_message_use, message_use_chosen, bot_enabled, bot_chosen)) => UserSettings {
                provider: provider_from(provider),
                off,
                allow_message_use,
                message_use_chosen,
                bot_enabled,
                bot_chosen,
            },
            None => UserSettings {
                provider: None,
                off: false,
                allow_message_use: None,
                message_use_chosen: false,
                bot_enabled: None,
                bot_chosen: false,
            },
        })
    }

    pub(crate) async fn ai_own_keys(&self, user_id: Uuid) -> AppResult<Vec<OwnKey>> {
        let rows: Vec<(i16, Vec<u8>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT provider, key_sealed, created_at FROM chat_ai_user_keys WHERE user_id = $1 ORDER BY provider",
        )
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(provider, sealed, created_at)| {
                let key = self.core.keys.open(USER_KEY_SEAL, &sealed).and_then(|key| String::from_utf8(key).ok())?;
                Some(OwnKey { provider: provider_from(provider)?, key, created_at })
            })
            .collect())
    }

    pub(crate) async fn save_ai_choice(&self, user_id: Uuid, provider: Option<AiProvider>, off: bool) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO chat_ai_user_settings (user_id, provider, off) VALUES ($1, $2, $3)
             ON CONFLICT (user_id) DO UPDATE SET provider = EXCLUDED.provider, off = EXCLUDED.off, updated_at = now()",
        )
        .bind(user_id)
        .bind(provider.map(provider_number).unwrap_or(0))
        .bind(off)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn save_ai_consent(&self, user_id: Uuid, allow: bool) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO chat_ai_user_settings (user_id, allow_message_use, message_use_chosen) VALUES ($1, $2, true)
             ON CONFLICT (user_id) DO UPDATE SET allow_message_use = EXCLUDED.allow_message_use,
                message_use_chosen = true, updated_at = now()",
        )
        .bind(user_id)
        .bind(allow)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn save_bot_enabled(&self, user_id: Uuid, enabled: bool) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO chat_ai_user_settings (user_id, bot_enabled, bot_chosen) VALUES ($1, $2, true)
             ON CONFLICT (user_id) DO UPDATE SET bot_enabled = EXCLUDED.bot_enabled, bot_chosen = true, updated_at = now()",
        )
        .bind(user_id)
        .bind(enabled)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn save_own_key(&self, user_id: Uuid, provider: AiProvider, key: &str) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO chat_ai_user_keys (user_id, provider, key_sealed, last4) VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, provider) DO UPDATE SET key_sealed = EXCLUDED.key_sealed, last4 = EXCLUDED.last4, created_at = now()",
        )
        .bind(user_id)
        .bind(provider_number(provider))
        .bind(self.core.keys.seal(USER_KEY_SEAL, key.as_bytes()))
        .bind(last4(key))
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn remove_own_key(&self, user_id: Uuid, provider: AiProvider) -> AppResult<()> {
        sqlx::query("DELETE FROM chat_ai_user_keys WHERE user_id = $1 AND provider = $2")
            .bind(user_id)
            .bind(provider_number(provider))
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Stores the age defaults of new accounts — the consent and
    /// 「Tokuly Bot と話す」: on for an adult, off for a minor
    /// (`tokuly_identity::age::minor_on`: no birthdate counts as one), judged
    /// by the age **when the account was created** — never by today's date,
    /// so a minor turning 18 does not start sharing (and nothing visible
    /// moves with a hidden birthday). A stored value, chosen or default,
    /// always wins. Idempotent: used for `identity.user.created` and for
    /// values a lost event left unstored. Today's birthdate stands for the
    /// creation day's: sign-up asks for it, and the accounts from before the
    /// question have the minors' values stored (migrations 0601, 0602).
    /// Returns what is stored: `(allow_message_use, bot_enabled)`.
    pub(crate) async fn store_age_defaults(&self, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, (bool, bool)>> {
        let accounts: Vec<(Uuid, Option<chrono::NaiveDate>, DateTime<Utc>)> =
            sqlx::query_as("SELECT id, birthdate, created_at FROM users WHERE id = ANY($1)")
                .bind(user_ids)
                .fetch_all(&self.core.db)
                .await?;
        let (ids, allow): (Vec<Uuid>, Vec<bool>) = accounts
            .into_iter()
            .map(|(id, birthdate, created_at)| (id, !age::minor_on(birthdate, created_at.date_naive())))
            .unzip();
        let rows: Vec<(Uuid, Option<bool>, Option<bool>)> = sqlx::query_as(
            "INSERT INTO chat_ai_user_settings (user_id, allow_message_use, bot_enabled)
             SELECT id, adult, adult FROM UNNEST($1::uuid[], $2::boolean[]) AS t (id, adult)
             ON CONFLICT (user_id) DO UPDATE
                SET allow_message_use = COALESCE(chat_ai_user_settings.allow_message_use, EXCLUDED.allow_message_use),
                    bot_enabled = COALESCE(chat_ai_user_settings.bot_enabled, EXCLUDED.bot_enabled)
             RETURNING user_id, allow_message_use, bot_enabled",
        )
        .bind(&ids)
        .bind(&allow)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.into_iter().map(|(id, allow, bot)| (id, (allow.unwrap_or(false), bot.unwrap_or(false)))).collect())
    }

    /// A minor gave the birthdate after the account was made
    /// (`identity.user.birthdate_given`): other people's AI requests leave
    /// their messages out and Tokuly Bot stops answering them from now, as
    /// from sign-up, unless the owner chose themselves. Idempotent.
    pub(crate) async fn apply_minor_defaults(&self, user_id: Uuid) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO chat_ai_user_settings AS s (user_id, allow_message_use, bot_enabled)
             SELECT id, false, false FROM users WHERE id = $1 AND deleted_at IS NULL
             ON CONFLICT (user_id) DO UPDATE SET
                allow_message_use = CASE WHEN s.message_use_chosen THEN s.allow_message_use ELSE false END,
                bot_enabled = CASE WHEN s.bot_chosen THEN s.bot_enabled ELSE false END,
                updated_at = now()
             WHERE (NOT s.message_use_chosen AND s.allow_message_use IS DISTINCT FROM false)
                OR (NOT s.bot_chosen AND s.bot_enabled IS DISTINCT FROM false)",
        )
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Whether each of `user_ids` lets other people's AI requests use their
    /// messages, as stored (see [`Self::store_age_defaults`]; what is
    /// not stored yet is stored now). Unknown users are left out.
    pub(crate) async fn ai_message_use_allowed(&self, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, bool>> {
        let rows: Vec<(Uuid, bool)> = sqlx::query_as(
            "SELECT user_id, allow_message_use FROM chat_ai_user_settings
             WHERE user_id = ANY($1) AND allow_message_use IS NOT NULL",
        )
        .bind(user_ids)
        .fetch_all(&self.core.db)
        .await?;
        let mut allowed: HashMap<Uuid, bool> = rows.into_iter().collect();
        let missing: Vec<Uuid> = user_ids.iter().copied().filter(|id| !allowed.contains_key(id)).collect();
        if !missing.is_empty() {
            allowed.extend(self.store_age_defaults(&missing).await?.into_iter().map(|(id, (allow, _))| (id, allow)));
        }
        Ok(allowed)
    }

    /// Whether Tokuly Bot answers `user_id`, as stored (see
    /// [`Self::store_age_defaults`]; not stored yet: stored now). An
    /// unknown account: no.
    pub(crate) async fn bot_enabled_for(&self, user_id: Uuid) -> AppResult<bool> {
        if let Some(enabled) = self.ai_user_settings(user_id).await?.bot_enabled {
            return Ok(enabled);
        }
        Ok(self.store_age_defaults(&[user_id]).await?.get(&user_id).is_some_and(|(_, bot)| *bot))
    }

    /// The caller's settings as the API shows them.
    pub(crate) async fn ai_settings_view(&self, user_id: Uuid) -> AppResult<pb::AssistSettings> {
        let instance = self.ai_instance().await?;
        let user = self.ai_user_settings(user_id).await?;
        let own_keys = self.ai_own_keys(user_id).await?;
        let default = instance.effective_default();
        let consent_default = !user.message_use_chosen;
        let allow_message_use = match user.allow_message_use {
            Some(allow) => allow,
            None => self.ai_message_use_allowed(&[user_id]).await?.get(&user_id).copied().unwrap_or(false),
        };
        let providers = instance
            .providers
            .iter()
            .filter(|settings| settings.configured())
            .map(|settings| {
                let own = own_keys.iter().any(|key| key.provider == settings.provider);
                pb::AssistProviderOption {
                    provider: settings.provider.into(),
                    recipient: settings.recipient(),
                    model: settings.model.clone(),
                    is_default: default == Some(settings.provider),
                    accepts_own_key: accepts_own_key(settings.provider),
                    usable: settings.usable_with(own && accepts_own_key(settings.provider)),
                    ..Default::default()
                }
            })
            .collect();
        let effective = route(&instance, &user, &own_keys).ok();
        let bot_enabled = match user.bot_enabled {
            Some(enabled) => enabled,
            None => self.bot_enabled_for(user_id).await?,
        };
        let bot = if self.bot_available(&instance) {
            Some(tokuly_identity::summary(&self.core, self.bot_id().await?).await?)
        } else {
            None
        };
        Ok(pb::AssistSettings {
            providers,
            chosen_provider: user.provider.unwrap_or(AiProvider::AI_PROVIDER_UNSPECIFIED).into(),
            off: user.off,
            effective_provider: effective
                .as_ref()
                .map(|route| route.settings.provider)
                .unwrap_or(AiProvider::AI_PROVIDER_UNSPECIFIED)
                .into(),
            effective_recipient: effective.as_ref().map(|route| route.settings.recipient()).unwrap_or_default(),
            own_keys: own_keys
                .iter()
                .map(|key| pb::OwnApiKey {
                    provider: key.provider.into(),
                    last4: last4(&key.key),
                    set_at: ts(key.created_at),
                    ..Default::default()
                })
                .collect(),
            allow_message_use,
            allow_message_use_is_default: consent_default,
            bot: bot.into(),
            bot_enabled,
            bot_enabled_is_default: !user.bot_chosen,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(provider: AiProvider, enabled: bool, key: bool) -> ProviderSettings {
        ProviderSettings {
            provider,
            enabled,
            model: "model".into(),
            base_url: "https://example.com".into(),
            api_key: key.then(|| "sk-operator-key".to_owned()),
            allow_private_host: false,
            from_environment: true,
            updated_at: None,
        }
    }

    fn user(provider: Option<AiProvider>) -> UserSettings {
        UserSettings {
            provider,
            off: false,
            allow_message_use: None,
            message_use_chosen: false,
            bot_enabled: None,
            bot_chosen: false,
        }
    }

    const ANTHROPIC: AiProvider = AiProvider::AI_PROVIDER_ANTHROPIC;
    const OPENAI: AiProvider = AiProvider::AI_PROVIDER_OPENAI;
    const COMPAT: AiProvider = AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE;

    #[test]
    fn a_user_gets_their_choice_or_the_default_and_never_another_company() {
        let instance = Instance {
            providers: vec![
                settings(ANTHROPIC, true, true),
                settings(OPENAI, true, false),
                settings(COMPAT, false, false),
            ],
            default: Some(OPENAI),
        };
        assert_eq!(instance.effective_default(), Some(OPENAI), "the default is the operator's pick while enabled");
        assert!(matches!(route(&instance, &user(None), &[]), Err(NoRoute::Disabled)), "the default lacks a key");
        let own = vec![OwnKey { provider: OPENAI, key: "sk-own-key-1234".into(), created_at: Utc::now() }];
        let routed = route(&instance, &user(None), &own).ok().expect("the user's key makes it usable");
        assert_eq!((routed.settings.provider, routed.api_key.as_deref()), (OPENAI, Some("sk-own-key-1234")));

        let routed = route(&instance, &user(Some(ANTHROPIC)), &own).ok().unwrap();
        assert_eq!(routed.api_key.as_deref(), Some("sk-operator-key"), "the own key belongs to OpenAI only");
        assert!(matches!(route(&instance, &user(Some(COMPAT)), &[]), Err(NoRoute::ChoiceUnavailable)));
        let off = UserSettings { off: true, ..user(Some(ANTHROPIC)) };
        assert!(matches!(route(&instance, &off, &[]), Err(NoRoute::TurnedOff)));

        let none = Instance { providers: vec![settings(ANTHROPIC, false, true)], default: None };
        assert_eq!(none.effective_default(), None);
        assert!(matches!(route(&none, &user(None), &[]), Err(NoRoute::Disabled)));
    }

    #[test]
    fn the_compatible_provider_needs_no_key_and_names_its_host() {
        let mut compat = settings(COMPAT, true, false);
        compat.base_url = "http://lmstudio.local:1234/v1".into();
        assert!(compat.usable_with(false));
        assert_eq!(compat.recipient(), "lmstudio.local");
        assert!(compat.allowance().is_none(), "no allowance unless granted");
        compat.allow_private_host = true;
        assert_eq!(compat.allowance().unwrap().host(), "lmstudio.local");
        let mut anthropic = settings(ANTHROPIC, true, true);
        anthropic.allow_private_host = true;
        assert!(anthropic.allowance().is_none(), "the allowance is for the compatible provider only");
        anthropic.base_url = "https://api.anthropic.com".into();
        assert_eq!(anthropic.recipient(), "Anthropic");
        // A proxy in front of the API receives the text: it is what is named.
        anthropic.base_url = "https://openrouter.ai/api".into();
        assert_eq!(anthropic.recipient(), "openrouter.ai");
        let mut openai = settings(OPENAI, true, true);
        openai.base_url = "https://api.openai.com/v1".into();
        assert_eq!(openai.recipient(), "OpenAI");
        openai.base_url = "https://Gateway.Example.com/openai/v1".into();
        assert_eq!(openai.recipient(), "gateway.example.com");
    }

    #[test]
    fn keys_are_shown_by_their_last_four_characters_only() {
        assert_eq!(last4("sk-ant-api03-secretABCD"), "ABCD");
        assert_eq!(last4("abc"), "abc");
        let config = settings(ANTHROPIC, true, true).to_config();
        assert!(config.api_key_set);
        assert_eq!(config.api_key_last4, "-key");
        assert!(!format!("{config:?}").contains("sk-operator"), "the key never appears in the API message");
    }

    #[test]
    fn base_urls_are_checked_against_the_outbound_guard() {
        let strict = Policy::default();
        let check =
            |provider, url: &str, allow| check_base_url(provider, url, allow, strict).map_err(|e| e.reason.to_string());
        assert_eq!(check(ANTHROPIC, "", false).unwrap(), "https://api.anthropic.com");
        assert_eq!(check(OPENAI, " https://api.openai.com/v1/ ", false).unwrap(), "https://api.openai.com/v1");
        assert_eq!(check(COMPAT, "", false).unwrap_err(), "BASE_URL_INVALID");
        assert_eq!(check(COMPAT, "ftp://lmstudio.local/v1", false).unwrap_err(), "BASE_URL_INVALID");
        assert_eq!(check(COMPAT, "http://user:pw@lmstudio.local/v1", false).unwrap_err(), "BASE_URL_INVALID");
        assert_eq!(check(COMPAT, "http://lmstudio.local/v1?x=1", false).unwrap_err(), "BASE_URL_INVALID");
        assert_eq!(check(COMPAT, "http://192.168.1.20:1234/v1", false).unwrap_err(), "BASE_URL_NOT_ALLOWED");
        assert_eq!(check(COMPAT, "http://192.168.1.20:1234/v1", true).unwrap(), "http://192.168.1.20:1234/v1");
        assert_eq!(check(COMPAT, "http://lmstudio.local:1234/v1", true).unwrap(), "http://lmstudio.local:1234/v1");
        assert_eq!(check(COMPAT, "http://169.254.169.254/v1", true).unwrap_err(), "BASE_URL_NOT_ALLOWED");
        assert_eq!(check(ANTHROPIC, "https://api.anthropic.com:8443", false).unwrap_err(), "BASE_URL_NOT_ALLOWED");
    }
}
