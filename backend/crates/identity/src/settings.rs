//! Per-account settings.
//!
//! Settings are stored as proto JSON in `users.privacy` / `users.appearance`.
//! A missing group (or an older row) falls back to [`default_privacy`] /
//! [`default_appearance`], so adding a setting never needs a data migration.
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::Visibility,
    identity::{AppearanceSettings, Audience, PrivacySettings, Settings},
};

pub fn default_privacy() -> PrivacySettings {
    PrivacySettings {
        friend_requests_from: Audience::AUDIENCE_EVERYONE.into(),
        friend_list_visible_to: Audience::AUDIENCE_FRIENDS.into(),
        footprints_visible_to: Audience::AUDIENCE_FRIENDS.into(),
        discoverable: true,
        nearby_discoverable: true,
        share_battery: true,
        share_activity: true,
        share_online_status: true,
        share_place_labels: true,
        share_now_playing: true,
        location_history_days: 0,
        record_history: true,
        share_views: true,
        direct_messages_from: Audience::AUDIENCE_FRIENDS.into(),
        calls_from: Audience::AUDIENCE_FRIENDS.into(),
        default_note_visibility: Visibility::VISIBILITY_PUBLIC.into(),
        birthday_visible_to: Audience::AUDIENCE_NOBODY.into(),
        birthday_show_year: false,
        ..Default::default()
    }
}

/// Below this age at sign-up an account starts with nearby discovery off.
pub const NEARBY_DISCOVERY_ADULT_AGE: u32 = 18;

/// The privacy settings an account signs up with: the defaults, except that
/// someone under [`NEARBY_DISCOVERY_ADULT_AGE`] is not discoverable by
/// strangers nearby until they turn it on. Stored as-is, so the choice stays
/// when they come of age; accounts that existed before keep what they had.
/// An empty object (adults) means "the defaults".
pub fn sign_up_privacy(birthdate: chrono::NaiveDate, today: chrono::NaiveDate) -> serde_json::Value {
    let minor = today.years_since(birthdate).is_none_or(|age| age < NEARBY_DISCOVERY_ADULT_AGE);
    if !minor {
        return serde_json::json!({});
    }
    let privacy = PrivacySettings { nearby_discoverable: false, ..default_privacy() };
    serde_json::to_value(&privacy).unwrap_or_else(|_| serde_json::json!({}))
}

pub fn default_appearance() -> AppearanceSettings {
    AppearanceSettings {
        theme: "system".into(),
        map_style: "tokuly".into(),
        accent_color: "#7c5cff".into(),
        dopamine_level: 3,
        sounds: true,
        haptics: true,
        distance_unit: "km".into(),
        autoplay: true,
        ..Default::default()
    }
}

/// An empty JSON object means "never customized".
fn is_unset(value: &serde_json::Value) -> bool {
    value.as_object().is_none_or(|o| o.is_empty())
}

pub fn privacy_from_json(value: &serde_json::Value) -> PrivacySettings {
    if is_unset(value) {
        return default_privacy();
    }
    serde_json::from_value(value.clone()).unwrap_or_else(|_| default_privacy())
}

pub fn appearance_from_json(value: &serde_json::Value) -> AppearanceSettings {
    if is_unset(value) {
        return default_appearance();
    }
    serde_json::from_value(value.clone()).unwrap_or_else(|_| default_appearance())
}

pub fn settings_from_json(privacy: &serde_json::Value, appearance: &serde_json::Value) -> Settings {
    Settings {
        privacy: privacy_from_json(privacy).into(),
        appearance: appearance_from_json(appearance).into(),
        ..Default::default()
    }
}

fn check_audience(value: &buffa::EnumValue<Audience>, field: &str) -> AppResult<()> {
    ensure!(
        value.as_known().is_some_and(|a| a != Audience::AUDIENCE_UNSPECIFIED),
        invalid,
        "SETTINGS_INVALID",
        "{field} must be a valid audience"
    );
    Ok(())
}

pub fn validate_privacy(p: &PrivacySettings) -> AppResult<()> {
    check_audience(&p.friend_requests_from, "friend_requests_from")?;
    check_audience(&p.friend_list_visible_to, "friend_list_visible_to")?;
    check_audience(&p.footprints_visible_to, "footprints_visible_to")?;
    check_audience(&p.direct_messages_from, "direct_messages_from")?;
    check_audience(&p.calls_from, "calls_from")?;
    ensure!(
        (0..=3650).contains(&p.location_history_days),
        invalid,
        "SETTINGS_INVALID",
        "location_history_days must be 0-3650"
    );
    ensure!(
        matches!(
            p.default_note_visibility.as_known(),
            Some(Visibility::VISIBILITY_PUBLIC | Visibility::VISIBILITY_FRIENDS | Visibility::VISIBILITY_PRIVATE)
        ),
        invalid,
        "SETTINGS_INVALID",
        "default_note_visibility must be public, friends or private"
    );
    // Unspecified: a client that does not know the setting (the stored one stays).
    ensure!(
        matches!(
            p.birthday_visible_to.as_known(),
            Some(
                Audience::AUDIENCE_UNSPECIFIED
                    | Audience::AUDIENCE_EVERYONE
                    | Audience::AUDIENCE_FRIENDS
                    | Audience::AUDIENCE_NOBODY
            )
        ),
        invalid,
        "SETTINGS_INVALID",
        "birthday_visible_to must be everyone, friends or nobody"
    );
    Ok(())
}

pub fn validate_appearance(a: &AppearanceSettings) -> AppResult<()> {
    ensure!(
        matches!(a.theme.as_str(), "system" | "light" | "dark"),
        invalid,
        "SETTINGS_INVALID",
        "theme must be system, light or dark"
    );
    ensure!(
        !a.map_style.is_empty()
            && a.map_style.len() <= 32
            && a.map_style.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        invalid,
        "SETTINGS_INVALID",
        "invalid map style"
    );
    let color_ok = a.accent_color.len() == 7
        && a.accent_color.starts_with('#')
        && a.accent_color[1..].bytes().all(|b| b.is_ascii_hexdigit());
    ensure!(color_ok, invalid, "SETTINGS_INVALID", "accent_color must be #rrggbb");
    ensure!((0..=5).contains(&a.dopamine_level), invalid, "SETTINGS_INVALID", "dopamine_level must be 0-5");
    ensure!(
        matches!(a.distance_unit.as_str(), "km" | "mi"),
        invalid,
        "SETTINGS_INVALID",
        "distance_unit must be km or mi"
    );
    Ok(())
}

/// A user's privacy settings (defaults when never customized or unknown user).
pub async fn privacy_of(core: &Core, user_id: Uuid) -> AppResult<PrivacySettings> {
    let value: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT privacy FROM users WHERE id = $1").bind(user_id).fetch_optional(&core.db).await?;
    Ok(value.as_ref().map(privacy_from_json).unwrap_or_else(default_privacy))
}

/// Privacy settings for many users at once, keyed by user ID.
pub async fn privacy_of_many(
    core: &Core,
    user_ids: &[Uuid],
) -> AppResult<std::collections::HashMap<Uuid, PrivacySettings>> {
    let rows: Vec<(Uuid, serde_json::Value)> =
        sqlx::query_as("SELECT id, privacy FROM users WHERE id = ANY($1)").bind(user_ids).fetch_all(&core.db).await?;
    Ok(rows.into_iter().map(|(id, v)| (id, privacy_from_json(&v))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_json_yields_defaults() {
        let p = privacy_from_json(&serde_json::json!({}));
        assert!(p.discoverable && p.record_history && p.share_battery);
        assert_eq!(appearance_from_json(&serde_json::Value::Null).dopamine_level, 3);
    }

    #[test]
    fn stored_settings_roundtrip_through_json() {
        let mut p = default_privacy();
        p.discoverable = false;
        p.location_history_days = 30;
        let json = serde_json::to_value(&p).unwrap();
        let back = privacy_from_json(&json);
        assert!(!back.discoverable);
        assert_eq!(back.location_history_days, 30);
        assert_eq!(back.friend_requests_from, Audience::AUDIENCE_EVERYONE);
    }

    #[test]
    fn validation_rejects_out_of_range_values() {
        let mut a = default_appearance();
        assert!(validate_appearance(&a).is_ok());
        a.dopamine_level = 9;
        assert!(validate_appearance(&a).is_err());
        let mut a = default_appearance();
        a.accent_color = "red".into();
        assert!(validate_appearance(&a).is_err());
        let mut p = default_privacy();
        assert!(validate_privacy(&p).is_ok());
        p.friend_requests_from = Audience::AUDIENCE_UNSPECIFIED.into();
        assert!(validate_privacy(&p).is_err());
    }
}
