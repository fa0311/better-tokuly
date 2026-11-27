//! OBS overlays: widgets a streamer adds to OBS as browser sources.
//!
//! A widget (chat box, alert box, goal bar, poll results) belongs to a
//! channel and is opened with an unguessable token instead of a sign-in:
//! the page `/overlay/<token>` calls `OverlayService.GetOverlay` and then
//! follows the realtime topic `overlay:<token>`, which only the token opens
//! ([`guard`]). Rotating the token makes the old page stop: its topic is no
//! longer published to. Tokens are stored hashed (lookups) and encrypted
//! (to show the owner the URL again).
//!
//! Styles: every kind has built-in presets ([`PRESETS`], returned by
//! `ListOverlayPresets` so clients do not hard-code them); a widget's CSS is
//! its preset followed by the owner's custom CSS. The class names the
//! presets style are the contract with the overlay page:
//! `.chat-body`, `.chat-message`, `.chat-avatar`, `.chat-message-name`,
//! `.chat-message-text`, `.chat-badge`, `.chat-gift-message`,
//! `.chat-gift-amount`; `.alert`, `.alert-title`, `.alert-message`;
//! `.goal`, `.goal-title`, `.goal-bar`, `.goal-fill`, `.goal-numbers`;
//! `.poll`, `.poll-question`, `.poll-option`, `.poll-bar`, `.poll-votes`.
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use tokuly_core::{prelude::*, realtime::TopicGuard};
use tokuly_proto::{
    common::LocalizedText,
    live::{
        Alert, AlertBoxSettings, AlertKind, ChatBoxSettings, GoalKind, GoalProgress, GoalSettings, Overlay,
        OverlayChanged, OverlayKind, OverlayPreset, OverlaySettings, PollWidgetSettings, live_event::Event,
        overlay_settings::Widget,
    },
};
use tokuly_video::channels::{self, ChannelRow};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Live,
    access::{self, Viewer},
    streams, tiers,
};

pub const MAX_OVERLAYS: i64 = 20;
pub const MAX_NAME: usize = 50;
pub const MAX_CSS: usize = 50_000;
pub const MAX_TEMPLATE: usize = 200;
pub const MAX_GOAL_TITLE: usize = 60;
pub const MAX_GOAL_TARGET: i64 = 1_000_000_000_000;

pub const KIND_CHAT_BOX: i32 = OverlayKind::OVERLAY_KIND_CHAT_BOX as i32;
pub const KIND_ALERT_BOX: i32 = OverlayKind::OVERLAY_KIND_ALERT_BOX as i32;
pub const KIND_GOAL: i32 = OverlayKind::OVERLAY_KIND_GOAL as i32;
pub const KIND_POLL: i32 = OverlayKind::OVERLAY_KIND_POLL as i32;

const TOKEN_LABEL: &str = "live/overlay-token";

pub(crate) const SEALED: tokuly_core::rekey::Sealed = tokuly_core::rekey::Sealed::bytes("live_overlays", "token_sealed", TOKEN_LABEL).hashed("token_hash");
/// How long the widgets of a channel are remembered before they are read again.
const TOPICS_TTL: Duration = Duration::from_secs(5);

/// The widgets of a channel as `(kind, topic)`, cached per process.
pub type TopicCache = parking_lot::Mutex<HashMap<Uuid, (Instant, Arc<Vec<(i32, String)>>)>>;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OverlayRow {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub owner_id: Uuid,
    pub kind: i32,
    pub name: String,
    pub preset: String,
    pub custom_css: String,
    /// `tokuly.live.v1.OverlaySettings` as proto JSON.
    pub settings: serde_json::Value,
    pub goal_baseline: i64,
    pub token_sealed: Vec<u8>,
    /// Names the widget's realtime topic; derived from the token.
    pub topic_key: String,
    pub created_at: Time,
    pub updated_at: Time,
}

const OVERLAY_COLUMNS: &str = "id, channel_id, owner_id, kind, name, preset, custom_css, settings, goal_baseline, token_sealed, topic_key, created_at, updated_at";

impl OverlayRow {
    pub fn settings(&self) -> OverlaySettings {
        serde_json::from_value(self.settings.clone()).unwrap_or_default()
    }

    fn token(&self, core: &Core) -> AppResult<String> {
        core.keys
            .open(TOKEN_LABEL, &self.token_sealed)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| AppError::internal(anyhow::anyhow!("token of overlay {} cannot be opened", self.id)))
    }

    /// The widget as the API returns it; `with_secret` adds the token and
    /// the page URL (for the owner).
    pub fn to_proto(&self, core: &Core, with_secret: bool) -> AppResult<Overlay> {
        let (token, url) = if with_secret {
            let token = self.token(core)?;
            let url = core.web_url(&format!("/overlay/{token}"));
            (token, url)
        } else {
            Default::default()
        };
        Ok(Overlay {
            id: self.id.to_string(),
            channel_id: self.channel_id.to_string(),
            kind: self.kind.into(),
            name: self.name.clone(),
            preset: self.preset.clone(),
            custom_css: self.custom_css.clone(),
            settings: Some(self.settings()).into(),
            token,
            url,
            created_at: ts(self.created_at),
            updated_at: ts(self.updated_at),
            ..Default::default()
        })
    }

    /// The realtime topic of this widget.
    pub fn topic(&self) -> String {
        topic_name(&self.topic_key)
    }

    /// The preset's CSS followed by the owner's.
    pub fn css(&self) -> String {
        let base = preset(self.kind, &self.preset).map(|preset| preset.css).unwrap_or_default();
        if self.custom_css.is_empty() { base.to_owned() } else { format!("{base}\n{}", self.custom_css) }
    }
}

fn not_found() -> AppError {
    AppError::not_found("OVERLAY_NOT_FOUND", "overlay not found")
}

/// The topic that carries the events of the widget with this topic key.
pub fn topic_name(topic_key: &str) -> String {
    format!("overlay:{topic_key}")
}

/// The topic key of a token: a keyed hash, so the topic's name says
/// nothing about the token and cannot be turned back into it.
fn topic_key_of(core: &Core, token: &str) -> String {
    core.keys.mac("live/overlay-topic", token.as_bytes())[..16].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A new token: sealed, hashed, and its topic key.
fn new_token(core: &Core) -> (Vec<u8>, String, String) {
    let token = tokuly_core::keys::random_token(24);
    let sealed = core.keys.seal(TOKEN_LABEL, token.as_bytes());
    let hash = core.keys.token_hash(&token);
    let topic_key = topic_key_of(core, &token);
    (sealed, hash, topic_key)
}

// ───────────────────────────── presets ─────────────────────────────

/// A built-in style.
pub struct Preset {
    pub id: &'static str,
    pub kind: i32,
    pub name_ja: &'static str,
    pub name_en: &'static str,
    pub css: &'static str,
}

/// What every preset starts with: a transparent page for OBS.
#[cfg(test)]
const BASE_CSS: &str = "html, body { margin: 0; overflow: hidden; background: transparent !important; }";

/// Every built-in style; the first of a kind is its default.
pub const PRESETS: &[Preset] = &[
    Preset {
        id: "bubble",
        kind: KIND_CHAT_BOX,
        name_ja: "吹き出し",
        name_en: "Bubbles",
        css: "html, body { margin: 0; overflow: hidden; background: transparent !important; }
.chat-body { display: flex; flex-direction: column; justify-content: flex-end; height: 100vh; padding: 12px 0; box-sizing: border-box; font-family: system-ui, sans-serif; }
.chat-message { display: flex; gap: 8px; width: fit-content; max-width: calc(100% - 16px); margin: 2px 0; padding: 4px 12px; animation: chat-in 240ms ease-out both; }
.chat-avatar { flex: 0 0 32px; width: 32px; height: 32px; object-fit: cover; border: 2px solid rgba(255, 255, 255, 0.95); border-radius: 50%; box-shadow: 0 2px 8px rgba(0, 0, 0, 0.28); }
.chat-message-content { min-width: 0; padding: 7px 11px; border-radius: 4px 14px 14px 14px; background: rgba(255, 255, 255, 0.94); box-shadow: 0 2px 8px rgba(0, 0, 0, 0.22); }
.chat-message-name { display: block; margin-bottom: 2px; color: #555555; font-size: 12px; font-weight: 700; }
.chat-badge { margin-left: 4px; padding: 0 5px; border-radius: 6px; background: #ff5c8a; color: #ffffff; font-size: 10px; }
.chat-message-text { color: #111111; font-size: 16px; font-weight: 600; line-height: 1.45; overflow-wrap: anywhere; }
.chat-gift-message { margin: 6px 12px; padding: 10px 12px; border-radius: 12px; box-shadow: 0 3px 12px rgba(0, 0, 0, 0.3); }
.chat-gift-amount { font-size: 15px; font-weight: 800; }
@keyframes chat-in { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: translateY(0); } }",
    },
    Preset {
        id: "simple",
        kind: KIND_CHAT_BOX,
        name_ja: "シンプルホワイト",
        name_en: "Simple white",
        css: "html, body { margin: 0; overflow: hidden; background: transparent !important; }
.chat-body { display: flex; flex-direction: column; justify-content: flex-end; height: 100vh; padding: 8px; box-sizing: border-box; font-family: system-ui, sans-serif; }
.chat-message { display: flex; gap: 6px; align-items: baseline; margin: 1px 0; color: #ffffff; text-shadow: 0 1px 3px rgba(0, 0, 0, 0.9), 0 0 2px rgba(0, 0, 0, 0.9); }
.chat-avatar { display: none; }
.chat-message-name { font-size: 14px; font-weight: 700; opacity: 0.85; }
.chat-badge { margin-left: 4px; font-size: 10px; }
.chat-message-text { font-size: 18px; font-weight: 700; overflow-wrap: anywhere; }
.chat-gift-message { margin: 4px 0; padding: 6px 10px; border-radius: 8px; text-shadow: none; }
.chat-gift-amount { font-weight: 800; }",
    },
    Preset {
        id: "neon",
        kind: KIND_CHAT_BOX,
        name_ja: "ネオン",
        name_en: "Neon",
        css: "html, body { margin: 0; overflow: hidden; background: transparent !important; }
.chat-body { display: flex; flex-direction: column; justify-content: flex-end; height: 100vh; padding: 10px; box-sizing: border-box; font-family: system-ui, sans-serif; }
.chat-message { display: flex; gap: 8px; margin: 3px 0; padding: 6px 12px; border: 1px solid #5cf2ff; border-radius: 10px; background: rgba(8, 10, 30, 0.72); box-shadow: 0 0 10px rgba(92, 242, 255, 0.55); }
.chat-avatar { width: 28px; height: 28px; border-radius: 50%; object-fit: cover; }
.chat-message-name { color: #ff7ae0; font-size: 13px; font-weight: 700; }
.chat-badge { margin-left: 4px; color: #ffe066; font-size: 10px; }
.chat-message-text { color: #e9fbff; font-size: 16px; overflow-wrap: anywhere; }
.chat-gift-message { border-color: #ffe066; box-shadow: 0 0 14px rgba(255, 224, 102, 0.7); }
.chat-gift-amount { color: #ffe066; font-weight: 800; }",
    },
    Preset {
        id: "pop",
        kind: KIND_ALERT_BOX,
        name_ja: "ポップ",
        name_en: "Pop",
        css: "html, body { margin: 0; overflow: hidden; background: transparent !important; }
.alert { display: flex; flex-direction: column; align-items: center; gap: 8px; margin: 24px auto; padding: 18px 28px; width: fit-content; max-width: 80%; border-radius: 24px; background: #ffffff; box-shadow: 0 8px 28px rgba(0, 0, 0, 0.35); font-family: system-ui, sans-serif; animation: alert-pop 420ms cubic-bezier(0.2, 1.4, 0.4, 1) both; }
.alert-title { color: #ff3d7f; font-size: 28px; font-weight: 900; text-align: center; }
.alert-message { color: #222222; font-size: 20px; font-weight: 600; text-align: center; overflow-wrap: anywhere; }
@keyframes alert-pop { from { opacity: 0; transform: scale(0.6); } to { opacity: 1; transform: scale(1); } }",
    },
    Preset {
        id: "banner",
        kind: KIND_ALERT_BOX,
        name_ja: "バナー",
        name_en: "Banner",
        css: "html, body { margin: 0; overflow: hidden; background: transparent !important; }
.alert { margin: 0; padding: 14px 24px; background: linear-gradient(90deg, rgba(20, 20, 30, 0.92), rgba(20, 20, 30, 0.6)); color: #ffffff; font-family: system-ui, sans-serif; animation: alert-slide 320ms ease-out both; }
.alert-title { font-size: 22px; font-weight: 800; }
.alert-message { font-size: 16px; opacity: 0.9; overflow-wrap: anywhere; }
@keyframes alert-slide { from { opacity: 0; transform: translateX(-24px); } to { opacity: 1; transform: translateX(0); } }",
    },
    Preset {
        id: "bar",
        kind: KIND_GOAL,
        name_ja: "バー",
        name_en: "Bar",
        css: "html, body { margin: 0; overflow: hidden; background: transparent !important; }
.goal { padding: 10px 14px; font-family: system-ui, sans-serif; color: #ffffff; text-shadow: 0 1px 3px rgba(0, 0, 0, 0.8); }
.goal-title { margin-bottom: 6px; font-size: 18px; font-weight: 800; }
.goal-bar { position: relative; height: 26px; border-radius: 13px; background: rgba(0, 0, 0, 0.45); overflow: hidden; }
.goal-fill { height: 100%; border-radius: 13px; background: linear-gradient(90deg, #ff5c8a, #ffb347); transition: width 600ms ease; }
.goal-numbers { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; font-size: 14px; font-weight: 800; }",
    },
    Preset {
        id: "minimal",
        kind: KIND_GOAL,
        name_ja: "ミニマル",
        name_en: "Minimal",
        css: "html, body { margin: 0; overflow: hidden; background: transparent !important; }
.goal { padding: 6px 8px; font-family: system-ui, sans-serif; color: #ffffff; }
.goal-title { font-size: 14px; font-weight: 700; }
.goal-bar { height: 6px; margin-top: 4px; border-radius: 3px; background: rgba(255, 255, 255, 0.3); overflow: hidden; }
.goal-fill { height: 100%; background: #ffffff; transition: width 600ms ease; }
.goal-numbers { margin-top: 2px; font-size: 12px; opacity: 0.85; }",
    },
    Preset {
        id: "card",
        kind: KIND_POLL,
        name_ja: "カード",
        name_en: "Card",
        css: "html, body { margin: 0; overflow: hidden; background: transparent !important; }
.poll { margin: 12px; padding: 14px 16px; border-radius: 16px; background: rgba(255, 255, 255, 0.95); box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3); font-family: system-ui, sans-serif; color: #111111; }
.poll-question { margin-bottom: 10px; font-size: 18px; font-weight: 800; }
.poll-option { position: relative; margin: 6px 0; padding: 6px 10px; border-radius: 8px; background: #eeeeee; font-size: 15px; font-weight: 600; overflow: hidden; }
.poll-bar { position: absolute; inset: 0 auto 0 0; background: rgba(255, 92, 138, 0.35); transition: width 400ms ease; }
.poll-votes { float: right; font-variant-numeric: tabular-nums; }",
    },
    Preset {
        id: "ghost",
        kind: KIND_POLL,
        name_ja: "すけすけ",
        name_en: "See-through",
        css: "html, body { margin: 0; overflow: hidden; background: transparent !important; }
.poll { margin: 8px; font-family: system-ui, sans-serif; color: #ffffff; text-shadow: 0 1px 3px rgba(0, 0, 0, 0.9); }
.poll-question { margin-bottom: 6px; font-size: 18px; font-weight: 800; }
.poll-option { position: relative; margin: 4px 0; padding: 4px 8px; border: 1px solid rgba(255, 255, 255, 0.6); border-radius: 6px; font-size: 15px; overflow: hidden; }
.poll-bar { position: absolute; inset: 0 auto 0 0; background: rgba(255, 255, 255, 0.28); transition: width 400ms ease; }
.poll-votes { float: right; font-variant-numeric: tabular-nums; }",
    },
];

/// The built-in style `id` of a kind.
pub fn preset(kind: i32, id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.kind == kind && preset.id == id)
}

/// The default style of a kind.
pub fn default_preset(kind: i32) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.kind == kind)
}

/// The presets as the API returns them.
pub fn presets_proto() -> Vec<OverlayPreset> {
    PRESETS
        .iter()
        .map(|preset| OverlayPreset {
            id: preset.id.to_owned(),
            kind: preset.kind.into(),
            name: Some(LocalizedText {
                ja: preset.name_ja.to_owned(),
                en: preset.name_en.to_owned(),
                ..Default::default()
            })
            .into(),
            css: preset.css.to_owned(),
            ..Default::default()
        })
        .collect()
}

// ───────────────────────────── validation ─────────────────────────────

/// The kind a client asked for (`OVERLAY_KIND_INVALID`).
pub fn validate_kind(kind: buffa::EnumValue<OverlayKind>) -> AppResult<i32> {
    match kind.as_known() {
        Some(OverlayKind::OVERLAY_KIND_UNSPECIFIED) | None => {
            Err(AppError::invalid("OVERLAY_KIND_INVALID", "unknown kind of overlay"))
        }
        Some(known) => Ok(known as i32),
    }
}

pub fn validate_name(input: &str) -> AppResult<String> {
    let name = input.split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!(
        (1..=MAX_NAME).contains(&name.graphemes(true).count()) && !name.chars().any(char::is_control),
        invalid,
        "OVERLAY_NAME_INVALID",
        "an overlay name is 1 to {MAX_NAME} characters"
    );
    Ok(name)
}

/// The preset to use: the one named, or the kind's default when none is.
pub fn validate_preset(kind: i32, id: &str) -> AppResult<String> {
    let found = if id.trim().is_empty() { default_preset(kind) } else { preset(kind, id.trim()) };
    found
        .map(|preset| preset.id.to_owned())
        .ok_or_else(|| AppError::invalid("OVERLAY_PRESET_UNKNOWN", "no such style for this kind of overlay"))
}

pub fn validate_css(input: &str) -> AppResult<String> {
    ensure!(
        input.chars().count() <= MAX_CSS,
        invalid,
        "OVERLAY_CSS_TOO_LONG",
        "custom CSS is at most {MAX_CSS} characters"
    );
    let lower = input.to_ascii_lowercase();
    // A stylesheet cannot close its own <style> element.
    ensure!(!lower.contains("</style"), invalid, "OVERLAY_SETTINGS_INVALID", "custom CSS cannot contain markup");
    // The widget runs inside the streamer's OBS and, by its URL, carries
    // their token: custom CSS may style it but not make it load anything
    // from elsewhere (which would tell that server who is streaming, and
    // leak the URL as a referrer). Escapes are refused because they can
    // spell any of the words below.
    const LOADERS: [&str; 7] = ["@import", "url(", "image-set(", "@font-face", "src:", "expression(", "\\"];
    let compact: String = lower.split_whitespace().collect();
    ensure!(
        !LOADERS.iter().any(|loader| compact.contains(loader)),
        invalid,
        "OVERLAY_CSS_UNSAFE",
        "custom CSS cannot load images, fonts or other stylesheets, or use escapes"
    );
    Ok(input.to_owned())
}

/// Checks a widget's settings against its kind and fills in defaults.
/// `previous` are the settings being replaced (a goal keeps its start).
pub fn validate_settings(
    kind: i32,
    input: Option<&OverlaySettings>,
    previous: Option<&OverlaySettings>,
) -> AppResult<OverlaySettings> {
    let bad = |message: &'static str| AppError::invalid("OVERLAY_SETTINGS_INVALID", message);
    let template = |text: &str| -> AppResult<String> {
        let text = text.trim();
        if text.graphemes(true).count() > MAX_TEMPLATE || text.chars().any(char::is_control) {
            return Err(bad("an alert text is at most 200 characters"));
        }
        Ok(text.to_owned())
    };
    let given = input.and_then(|settings| settings.widget.as_ref());
    let widget = match (kind, given) {
        (KIND_CHAT_BOX, None) => Widget::ChatBox(Box::new(ChatBoxSettings { max_messages: 20, ..Default::default() })),
        (KIND_CHAT_BOX, Some(Widget::ChatBox(chat))) => {
            if !(0..=50).contains(&chat.max_messages) || !(0..=600).contains(&chat.fade_seconds) {
                return Err(bad("a chat box shows 1 to 50 lines and fades after at most 600 seconds"));
            }
            let mut chat = chat.as_ref().clone();
            if chat.max_messages == 0 {
                chat.max_messages = 20;
            }
            Widget::ChatBox(Box::new(chat))
        }
        (KIND_ALERT_BOX, None) => {
            Widget::AlertBox(Box::new(AlertBoxSettings { duration_seconds: 6, ..Default::default() }))
        }
        (KIND_ALERT_BOX, Some(Widget::AlertBox(alert))) => {
            if alert.duration_seconds != 0 && !(2..=30).contains(&alert.duration_seconds) {
                return Err(bad("an alert stays for 2 to 30 seconds"));
            }
            if !(0..=tiers::MAX_GIFT_COINS).contains(&alert.min_gift_coins) {
                return Err(bad("the smallest gift to announce is out of range"));
            }
            Widget::AlertBox(Box::new(AlertBoxSettings {
                duration_seconds: if alert.duration_seconds == 0 { 6 } else { alert.duration_seconds },
                gift_template: template(&alert.gift_template)?,
                subscriber_template: template(&alert.subscriber_template)?,
                raid_template: template(&alert.raid_template)?,
                ..alert.as_ref().clone()
            }))
        }
        (KIND_GOAL, given) => {
            let goal = match given {
                None => GoalSettings { kind: GoalKind::GOAL_KIND_COINS.into(), target: 1_000, ..Default::default() },
                Some(Widget::Goal(goal)) => goal.as_ref().clone(),
                Some(_) => return Err(bad("these settings belong to another kind of overlay")),
            };
            let goal_kind = match goal.kind.as_known() {
                Some(GoalKind::GOAL_KIND_UNSPECIFIED) => GoalKind::GOAL_KIND_COINS,
                Some(known) => known,
                None => return Err(bad("unknown kind of goal")),
            };
            let title = goal.title.trim().to_owned();
            if title.graphemes(true).count() > MAX_GOAL_TITLE || title.chars().any(char::is_control) {
                return Err(bad("a goal title is at most 60 characters"));
            }
            if !(1..=MAX_GOAL_TARGET).contains(&goal.target) {
                return Err(bad("a goal needs a target of at least 1"));
            }
            // Counting continues from where it began unless the goal is new.
            let started_at = match previous.and_then(|settings| settings.widget.as_ref()) {
                Some(Widget::Goal(old)) if old.kind == goal_kind && old.started_at.is_set() => old.started_at.clone(),
                _ => ts(now()),
            };
            Widget::Goal(Box::new(GoalSettings {
                kind: goal_kind.into(),
                title,
                target: goal.target,
                started_at,
                hide_numbers: goal.hide_numbers,
                ..Default::default()
            }))
        }
        (KIND_POLL, None) => Widget::Poll(Box::new(PollWidgetSettings::default())),
        (KIND_POLL, Some(Widget::Poll(poll))) => Widget::Poll(poll.clone()),
        _ => return Err(bad("these settings belong to another kind of overlay")),
    };
    Ok(OverlaySettings { widget: Some(widget), ..Default::default() })
}

fn settings_json(settings: &OverlaySettings) -> AppResult<serde_json::Value> {
    serde_json::to_value(settings).map_err(|e| AppError::internal(anyhow::anyhow!("overlay settings encode: {e}")))
}

// ───────────────────────────── lookups ─────────────────────────────

pub async fn find_owned(core: &Core, owner_id: Uuid, overlay_id: &str) -> AppResult<OverlayRow> {
    let Ok(id) = Uuid::parse_str(overlay_id.trim()) else { return Err(not_found()) };
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {OVERLAY_COLUMNS} FROM live_overlays WHERE id = $1 AND owner_id = $2"
    )))
    .bind(id)
    .bind(owner_id)
    .fetch_optional(&core.db)
    .await?
    .ok_or_else(not_found)
}

pub async fn find_by_token(core: &Core, token: &str) -> AppResult<Option<OverlayRow>> {
    if token.is_empty() || token.len() > 128 {
        return Ok(None);
    }
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {OVERLAY_COLUMNS} FROM live_overlays WHERE token_hash = ANY($1)"
    )))
    .bind(core.keys.token_hashes(token))
    .fetch_optional(&core.db)
    .await?)
}

pub async fn of_channel(core: &Core, channel_id: Uuid) -> AppResult<Vec<OverlayRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {OVERLAY_COLUMNS} FROM live_overlays WHERE channel_id = $1 ORDER BY id"
    )))
    .bind(channel_id)
    .fetch_all(&core.db)
    .await?)
}

/// The widgets of a channel as `(kind, topic)`. Remembered for a few
/// seconds: this is asked for every chat message.
pub async fn topics(live: &Live, channel_id: Uuid) -> AppResult<Arc<Vec<(i32, String)>>> {
    if let Some((read_at, cached)) = live.overlay_topics.lock().get(&channel_id)
        && read_at.elapsed() < TOPICS_TTL
    {
        return Ok(cached.clone());
    }
    let rows = of_channel(&live.core, channel_id).await?;
    let widgets: Arc<Vec<(i32, String)>> = Arc::new(rows.iter().map(|row| (row.kind, row.topic())).collect());
    let mut cache = live.overlay_topics.lock();
    // Channels come and go; do not remember them forever.
    cache.retain(|_, (read_at, _)| read_at.elapsed() < TOPICS_TTL);
    cache.insert(channel_id, (Instant::now(), widgets.clone()));
    Ok(widgets)
}

fn forget_topics(live: &Live, channel_id: Uuid) {
    live.overlay_topics.lock().remove(&channel_id);
}

// ───────────────────────────── goals ─────────────────────────────

/// Where a goal widget's goal stands; `None` for other kinds of widgets.
pub async fn goal_progress(core: &Core, row: &OverlayRow) -> AppResult<Option<GoalProgress>> {
    let settings = row.settings();
    let Some(Widget::Goal(goal)) = settings.widget else { return Ok(None) };
    let current = match goal.kind.as_known() {
        Some(GoalKind::GOAL_KIND_SUBSCRIBERS) => {
            let subscribers =
                channels::summaries(core, &[row.channel_id]).await?.remove(&row.channel_id).map(|c| c.subscriber_count);
            (subscribers.unwrap_or(0) - row.goal_baseline).max(0)
        }
        _ => {
            let since = goal.started_at.as_option().map(tokuly_core::time::from_ts).unwrap_or(row.created_at);
            sqlx::query_scalar(
                "SELECT COALESCE(sum(coins), 0)::bigint FROM live_gifts WHERE channel_id = $1 AND status = 2 AND created_at >= $2",
            )
            .bind(row.channel_id)
            .bind(since)
            .fetch_one(&core.db)
            .await?
        }
    };
    Ok(Some(GoalProgress {
        overlay_id: row.id.to_string(),
        kind: goal.kind,
        title: goal.title.clone(),
        current,
        target: goal.target,
        ..Default::default()
    }))
}

/// Pushes the progress of every goal widget of a channel to that widget.
pub async fn push_goal_progress(live: &Live, channel_id: Uuid) {
    let pushed = async {
        for row in of_channel(&live.core, channel_id).await?.iter().filter(|row| row.kind == KIND_GOAL) {
            if let Some(progress) = goal_progress(&live.core, row).await? {
                let topic = row.topic();
                let event = tokuly_proto::live::LiveEvent {
                    event: Some(Event::GoalProgress(Box::new(progress))),
                    ..Default::default()
                };
                live.core.rt.notify_topic(&topic, &event).await;
            }
        }
        AppResult::Ok(())
    };
    if let Err(error) = pushed.await {
        tracing::warn!(%channel_id, %error, "could not push goal progress");
    }
}

async fn subscriber_count(core: &Core, channel_id: Uuid) -> AppResult<i64> {
    Ok(channels::summaries(core, &[channel_id]).await?.remove(&channel_id).map(|c| c.subscriber_count).unwrap_or(0))
}

// ───────────────────────────── changes ─────────────────────────────

/// What a caller wants a widget to look like (`None` = leave as it is).
#[derive(Debug, Default)]
pub struct OverlayEdit {
    pub name: Option<String>,
    pub preset: Option<String>,
    pub custom_css: Option<String>,
    pub settings: Option<OverlaySettings>,
    pub reset_goal: bool,
}

impl Live {
    /// Adds a widget to a channel.
    pub async fn add_overlay(&self, channel: &ChannelRow, kind: i32, edit: OverlayEdit) -> AppResult<OverlayRow> {
        let core = &self.core;
        let name = validate_name(edit.name.as_deref().unwrap_or_default())?;
        let preset = validate_preset(kind, edit.preset.as_deref().unwrap_or_default())?;
        let css = validate_css(edit.custom_css.as_deref().unwrap_or_default())?;
        let settings = validate_settings(kind, edit.settings.as_ref(), None)?;
        let baseline = if kind == KIND_GOAL { subscriber_count(core, channel.id).await? } else { 0 };
        let (sealed, hash, topic_key) = new_token(core);
        let mut tx = core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("live:overlays:{}", channel.id.simple()))
            .execute(&mut *tx)
            .await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM live_overlays WHERE channel_id = $1")
            .bind(channel.id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            count < MAX_OVERLAYS,
            precondition,
            "OVERLAY_LIMIT_REACHED",
            "a channel has at most {MAX_OVERLAYS} overlays"
        );
        let row: OverlayRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO live_overlays (channel_id, owner_id, kind, name, preset, custom_css, settings, goal_baseline, token_hash, token_sealed, topic_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING {OVERLAY_COLUMNS}"
        )))
        .bind(channel.id)
        .bind(channel.owner_id)
        .bind(kind)
        .bind(&name)
        .bind(&preset)
        .bind(&css)
        .bind(settings_json(&settings)?)
        .bind(baseline)
        .bind(&hash)
        .bind(&sealed)
        .bind(&topic_key)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        forget_topics(self, channel.id);
        Ok(row)
    }

    /// Applies `edit` to a widget and tells the open widget to reload.
    pub async fn change_overlay(&self, row: &OverlayRow, edit: OverlayEdit) -> AppResult<OverlayRow> {
        let core = &self.core;
        let name = edit.name.as_deref().map(validate_name).transpose()?;
        let preset = edit.preset.as_deref().map(|id| validate_preset(row.kind, id)).transpose()?;
        let css = edit.custom_css.as_deref().map(validate_css).transpose()?;
        let previous = row.settings();
        let mut settings = match &edit.settings {
            Some(settings) => Some(validate_settings(row.kind, Some(settings), Some(&previous))?),
            None => None,
        };
        let mut baseline = None;
        if row.kind == KIND_GOAL {
            let old_kind = match &previous.widget {
                Some(Widget::Goal(goal)) => Some(goal.kind),
                _ => None,
            };
            let mut current = settings.clone().unwrap_or(previous.clone());
            let new_kind = match &current.widget {
                Some(Widget::Goal(goal)) => Some(goal.kind),
                _ => None,
            };
            // A new kind of goal, or an explicit reset, starts counting from now.
            if edit.reset_goal || old_kind != new_kind {
                if let Some(Widget::Goal(goal)) = &mut current.widget {
                    goal.started_at = ts(now());
                }
                baseline = Some(subscriber_count(core, row.channel_id).await?);
                settings = Some(current);
            }
        }
        let updated: OverlayRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE live_overlays SET name = COALESCE($2, name), preset = COALESCE($3, preset), custom_css = COALESCE($4, custom_css),
                    settings = COALESCE($5, settings), goal_baseline = COALESCE($6, goal_baseline)
             WHERE id = $1 RETURNING {OVERLAY_COLUMNS}"
        )))
        .bind(row.id)
        .bind(&name)
        .bind(&preset)
        .bind(&css)
        .bind(settings.as_ref().map(settings_json).transpose()?)
        .bind(baseline)
        .fetch_optional(&core.db)
        .await?
        .ok_or_else(not_found)?;
        self.push_overlay_changed(&updated, false).await;
        Ok(updated)
    }

    /// Tells the page behind `row`'s topic (as it was when `row` was read).
    async fn push_overlay_changed(&self, row: &OverlayRow, revoked: bool) {
        let change = OverlayChanged { overlay_id: row.id.to_string(), revoked, ..Default::default() };
        let event = tokuly_proto::live::LiveEvent {
            event: Some(Event::OverlayChanged(Box::new(change))),
            ..Default::default()
        };
        self.core.rt.notify_topic(&row.topic(), &event).await;
        if revoked {
            // After the event: the page behind the old token hears nothing more.
            self.core.rt.close_topic(&row.topic()).await;
        }
    }

    pub async fn remove_overlay(&self, row: &OverlayRow) -> AppResult<()> {
        sqlx::query("DELETE FROM live_overlays WHERE id = $1").bind(row.id).execute(&self.core.db).await?;
        forget_topics(self, row.channel_id);
        self.push_overlay_changed(row, true).await;
        Ok(())
    }

    /// Gives a widget a new token; the page behind the old one is told to stop.
    pub async fn renew_overlay_token(&self, row: &OverlayRow) -> AppResult<OverlayRow> {
        let core = &self.core;
        let (sealed, hash, topic_key) = new_token(core);
        let updated: OverlayRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE live_overlays SET token_hash = $2, token_sealed = $3, topic_key = $4 WHERE id = $1 RETURNING {OVERLAY_COLUMNS}"
        )))
        .bind(row.id)
        .bind(&hash)
        .bind(&sealed)
        .bind(&topic_key)
        .fetch_optional(&core.db)
        .await?
        .ok_or_else(not_found)?;
        forget_topics(self, row.channel_id);
        self.push_overlay_changed(row, true).await;
        Ok(updated)
    }

    /// Plays a sample alert on the channel's alert boxes. Returns how many
    /// there are.
    pub async fn play_test_alert(&self, channel: &ChannelRow, kind: AlertKind) -> AppResult<i32> {
        let alert = match kind {
            AlertKind::ALERT_KIND_SUBSCRIBER => {
                Alert { kind: kind.into(), name: "テストユーザー".to_owned(), test: true, ..Default::default() }
            }
            AlertKind::ALERT_KIND_RAID => Alert {
                kind: kind.into(),
                name: "テストチャンネル".to_owned(),
                viewer_count: 42,
                test: true,
                ..Default::default()
            },
            _ => Alert {
                kind: AlertKind::ALERT_KIND_GIFT.into(),
                name: "テストユーザー".to_owned(),
                coins: 500,
                tier: tiers::tier_for(500).map(|tier| tier.level).unwrap_or(1),
                message: "テストアラートだよ!".to_owned(),
                test: true,
                ..Default::default()
            },
        };
        forget_topics(self, channel.id);
        let boxes = topics(self, channel.id).await?.iter().filter(|(kind, _)| *kind == KIND_ALERT_BOX).count();
        self.push_to_overlays(channel.id, &[OverlayKind::OVERLAY_KIND_ALERT_BOX], Event::Alert(Box::new(alert))).await;
        Ok(boxes as i32)
    }
}

// ───────────────────────────── topic guards ─────────────────────────────

/// Guard of `overlay:<token>` topics: the token is the key.
struct OverlayGuard {
    core: Core,
}

#[async_trait]
impl TopicGuard for OverlayGuard {
    async fn may_subscribe(&self, _subscriber: Option<Uuid>, topic: &str) -> AppResult<bool> {
        let Some(topic_key) = topic.strip_prefix("overlay:") else { return Ok(false) };
        Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM live_overlays WHERE topic_key = $1)")
            .bind(topic_key)
            .fetch_one(&self.core.db)
            .await?)
    }
}

/// Guard of `live:<stream id>` topics: whoever may watch the stream may
/// follow it (guests included for streams open to everyone).
struct StreamGuard {
    core: Core,
}

#[async_trait]
impl TopicGuard for StreamGuard {
    async fn may_subscribe(&self, subscriber: Option<Uuid>, topic: &str) -> AppResult<bool> {
        let Some(stream_id) = topic.strip_prefix("live:").and_then(|id| Uuid::parse_str(id).ok()) else {
            return Ok(false);
        };
        let Some(stream) = streams::find_by_id(&self.core, stream_id).await? else { return Ok(false) };
        let viewer = Viewer::load(&self.core, subscriber).await?;
        access::can_watch_now(&self.core, &viewer, &stream.facts()).await
    }
}

/// Registers the guards of the `live` and `overlay` topics. They must be in
/// place in every process, the gateway's included.
pub fn register_guards(core: &Core) {
    core.topic_guards.register("live", Arc::new(StreamGuard { core: core.clone() }));
    core.topic_guards.register("overlay", Arc::new(OverlayGuard { core: core.clone() }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_presets_and_a_default() {
        for kind in [KIND_CHAT_BOX, KIND_ALERT_BOX, KIND_GOAL, KIND_POLL] {
            let default = default_preset(kind).expect("a default preset");
            assert_eq!(validate_preset(kind, "").unwrap(), default.id);
            assert_eq!(validate_preset(kind, default.id).unwrap(), default.id);
            assert_eq!(validate_preset(kind, "nope").unwrap_err().reason, "OVERLAY_PRESET_UNKNOWN");
        }
        // A chat style is not a goal style.
        assert_eq!(validate_preset(KIND_GOAL, "bubble").unwrap_err().reason, "OVERLAY_PRESET_UNKNOWN");
        let mut ids: Vec<(i32, &str)> = PRESETS.iter().map(|preset| (preset.kind, preset.id)).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len(), "preset IDs are unique per kind");
        assert!(PRESETS.iter().all(|preset| preset.css.starts_with(BASE_CSS)));
        assert_eq!(presets_proto().len(), PRESETS.len());
    }

    #[test]
    fn settings_must_match_the_kind_and_get_defaults() {
        let chat = validate_settings(KIND_CHAT_BOX, None, None).unwrap();
        assert!(matches!(chat.widget, Some(Widget::ChatBox(ref c)) if c.max_messages == 20));
        let alert = validate_settings(KIND_ALERT_BOX, None, None).unwrap();
        assert!(matches!(alert.widget, Some(Widget::AlertBox(ref a)) if a.duration_seconds == 6));
        let goal = validate_settings(KIND_GOAL, None, None).unwrap();
        assert!(matches!(goal.widget, Some(Widget::Goal(ref g)) if g.target == 1_000 && g.started_at.is_set()));
        assert!(matches!(validate_settings(KIND_POLL, None, None).unwrap().widget, Some(Widget::Poll(_))));

        let wrong = OverlaySettings { widget: Some(Widget::Poll(Box::default())), ..Default::default() };
        for kind in [KIND_CHAT_BOX, KIND_ALERT_BOX, KIND_GOAL] {
            assert_eq!(validate_settings(kind, Some(&wrong), None).unwrap_err().reason, "OVERLAY_SETTINGS_INVALID");
        }
        let too_many = OverlaySettings {
            widget: Some(Widget::ChatBox(Box::new(ChatBoxSettings { max_messages: 51, ..Default::default() }))),
            ..Default::default()
        };
        assert_eq!(
            validate_settings(KIND_CHAT_BOX, Some(&too_many), None).unwrap_err().reason,
            "OVERLAY_SETTINGS_INVALID"
        );
        let no_target = OverlaySettings {
            widget: Some(Widget::Goal(Box::new(GoalSettings { target: 0, ..Default::default() }))),
            ..Default::default()
        };
        assert_eq!(
            validate_settings(KIND_GOAL, Some(&no_target), None).unwrap_err().reason,
            "OVERLAY_SETTINGS_INVALID"
        );
    }

    #[test]
    fn a_goal_keeps_counting_from_its_start_when_edited() {
        let first = validate_settings(KIND_GOAL, None, None).unwrap();
        let Some(Widget::Goal(original)) = &first.widget else { panic!("a goal") };
        let edited = OverlaySettings {
            widget: Some(Widget::Goal(Box::new(GoalSettings {
                kind: GoalKind::GOAL_KIND_COINS.into(),
                title: "新しいマイク".into(),
                target: 5_000,
                ..Default::default()
            }))),
            ..Default::default()
        };
        let second = validate_settings(KIND_GOAL, Some(&edited), Some(&first)).unwrap();
        let Some(Widget::Goal(updated)) = &second.widget else { panic!("a goal") };
        assert_eq!(updated.started_at, original.started_at);
        assert_eq!((updated.title.as_str(), updated.target), ("新しいマイク", 5_000));
    }

    #[test]
    fn names_and_css_have_limits() {
        assert_eq!(validate_name("  縦配信用   チャット ").unwrap(), "縦配信用 チャット");
        assert_eq!(validate_name("").unwrap_err().reason, "OVERLAY_NAME_INVALID");
        assert_eq!(validate_name(&"x".repeat(51)).unwrap_err().reason, "OVERLAY_NAME_INVALID");
        assert!(validate_css(".chat-message { color: red; }").is_ok());
        assert_eq!(validate_css(&"a".repeat(MAX_CSS + 1)).unwrap_err().reason, "OVERLAY_CSS_TOO_LONG");
        assert_eq!(validate_css("x {} </STYLE><script>").unwrap_err().reason, "OVERLAY_SETTINGS_INVALID");
        assert_eq!(topic_name("abc"), "overlay:abc");
        // Remote loads and escapes that could hide them are refused.
        for css in [
            "@import url(https://evil.example/x.css);",
            "@IMPORT 'https://evil.example/x.css';",
            ".chat-body { background: url(https://evil.example/pixel.png); }",
            ".chat-body { background: URL( 'https://evil.example/pixel.png' ); }",
            ".chat-body { background: image-set('https://evil.example/a.png' 1x); }",
            ".a { background: \\75rl(https://evil.example/x); }",
            "@font-face { font-family: x; src: local(x); }",
            ".a { behavior: expression(alert(1)); }",
        ] {
            assert_eq!(validate_css(css).unwrap_err().reason, "OVERLAY_CSS_UNSAFE", "{css}");
        }
        assert!(
            validate_css(
                ".chat-message-text { color: #fff; font-size: 20px; } /* ok */ @keyframes in { from { opacity: 0 } }"
            )
            .is_ok()
        );
    }
}
