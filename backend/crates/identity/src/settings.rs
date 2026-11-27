//! Per-account settings.
//!
//! Settings are stored as proto JSON in `users.privacy` / `users.appearance`.
//! A missing group (or an older row) falls back to [`default_privacy`] /
//! [`default_appearance`], so adding a setting never needs a data migration.
//!
//! Accounts under 18 start with [`minor_defaults`] (stored, see
//! [`crate::age`]) and while they are minors their discovery and friend
//! requests reach no further than friends of friends: [`within_minor_limits`]
//! on every read through [`privacy_of`] / [`privacy_of_many`], and
//! [`check_minor_limits`] on every change.
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
        discoverable_to: Audience::AUDIENCE_EVERYONE.into(),
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

/// `privacy` with the defaults of an account under 18 in place: found and
/// asked to be friends by friends of friends only (no search, no address
/// books), notes for followers, not discoverable by strangers nearby until
/// they turn it on; the year of the birthday not shown to everyone.
pub fn minor_defaults(privacy: PrivacySettings) -> PrivacySettings {
    let birthday_show_year =
        privacy.birthday_show_year && crate::birthdays::audience_of(&privacy) != Audience::AUDIENCE_EVERYONE;
    PrivacySettings {
        birthday_show_year,
        discoverable_to: Audience::AUDIENCE_FRIENDS_OF_FRIENDS.into(),
        friend_requests_from: Audience::AUDIENCE_FRIENDS_OF_FRIENDS.into(),
        default_note_visibility: Visibility::VISIBILITY_FOLLOWERS.into(),
        nearby_discoverable: false,
        ..privacy
    }
}

/// The privacy settings an account signs up with: the defaults, or for a
/// minor (by the age on the day of sign-up) [`minor_defaults`], stored as-is
/// so they stay when the account comes of age. An empty object (adults)
/// means "the defaults".
pub fn sign_up_privacy(minor: bool) -> serde_json::Value {
    if !minor {
        return serde_json::json!({});
    }
    serde_json::to_value(minor_defaults(default_privacy())).unwrap_or_else(|_| serde_json::json!({}))
}

/// The audiences a minor may not choose for discovery and friend requests.
fn too_wide_for_minors(audience: &buffa::EnumValue<Audience>) -> bool {
    audience.as_known() == Some(Audience::AUDIENCE_EVERYONE)
}

/// `privacy` as it applies to an account under 18: whatever was stored
/// before the limits (or before the account gave its birthdate) reaches no
/// further than friends of friends.
pub fn within_minor_limits(mut privacy: PrivacySettings) -> PrivacySettings {
    for audience in [&mut privacy.discoverable_to, &mut privacy.friend_requests_from] {
        if too_wide_for_minors(audience) {
            *audience = Audience::AUDIENCE_FRIENDS_OF_FRIENDS.into();
        }
    }
    privacy
}

/// A minor's change may not widen discovery or friend requests past friends
/// of friends.
pub fn check_minor_limits(privacy: &PrivacySettings, minor: bool) -> AppResult<()> {
    ensure!(
        !(minor
            && (too_wide_for_minors(&privacy.discoverable_to) || too_wide_for_minors(&privacy.friend_requests_from))),
        invalid,
        "SETTING_RESTRICTED",
        "under 18, discovery and friend requests reach friends of friends at most"
    );
    Ok(())
}

/// Strangers may find the account: user search, address-book matching and
/// every kind of "people you may know".
pub fn findable_by_anyone(privacy: &PrivacySettings) -> bool {
    privacy.discoverable_to.as_known() == Some(Audience::AUDIENCE_EVERYONE)
}

/// The account may be suggested to friends of its friends.
pub fn findable_by_friends_of_friends(privacy: &PrivacySettings) -> bool {
    matches!(
        privacy.discoverable_to.as_known(),
        Some(Audience::AUDIENCE_EVERYONE | Audience::AUDIENCE_FRIENDS_OF_FRIENDS)
    )
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
    serde_json::from_value(with_discoverable_to(value)).unwrap_or_else(|_| default_privacy())
}

/// Settings stored before `discoverable_to` had the switch `discoverable`
/// instead (left out of the JSON when off): on means everyone, off nobody.
fn with_discoverable_to(value: &serde_json::Value) -> serde_json::Value {
    let mut value = value.clone();
    if let Some(fields) = value.as_object_mut() {
        let on = fields.remove("discoverable").is_some_and(|on| on.as_bool() == Some(true));
        if !fields.contains_key("discoverableTo") {
            let audience = if on { "AUDIENCE_EVERYONE" } else { "AUDIENCE_NOBODY" };
            fields.insert("discoverableTo".into(), audience.into());
        }
    }
    value
}

pub fn appearance_from_json(value: &serde_json::Value) -> AppearanceSettings {
    if is_unset(value) {
        return default_appearance();
    }
    serde_json::from_value(value.clone()).unwrap_or_else(|_| default_appearance())
}

/// The stored privacy settings as they apply to an account that is (`minor`)
/// or is not under 18.
pub fn privacy_as_applied(value: &serde_json::Value, minor: bool) -> PrivacySettings {
    let privacy = privacy_from_json(value);
    if minor { within_minor_limits(privacy) } else { privacy }
}

/// What the settings screens show: the privacy settings as they apply.
pub fn settings_from_json(privacy: &serde_json::Value, appearance: &serde_json::Value, minor: bool) -> Settings {
    Settings {
        privacy: privacy_as_applied(privacy, minor).into(),
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
    ensure!(
        matches!(
            p.discoverable_to.as_known(),
            Some(
                Audience::AUDIENCE_UNSPECIFIED
                    | Audience::AUDIENCE_EVERYONE
                    | Audience::AUDIENCE_FRIENDS_OF_FRIENDS
                    | Audience::AUDIENCE_NOBODY
            )
        ),
        invalid,
        "SETTINGS_INVALID",
        "discoverable_to must be everyone, friends of friends or nobody"
    );
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
            Some(
                Visibility::VISIBILITY_PUBLIC
                    | Visibility::VISIBILITY_FOLLOWERS
                    | Visibility::VISIBILITY_FRIENDS
                    | Visibility::VISIBILITY_PRIVATE
            )
        ),
        invalid,
        "SETTINGS_INVALID",
        "default_note_visibility must be public, followers, friends or private"
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

/// A user's privacy settings as they apply today: the stored ones (or the
/// defaults), within the limits for minors for an account under 18.
/// Defaults for an unknown user.
pub async fn privacy_of(core: &Core, user_id: Uuid) -> AppResult<PrivacySettings> {
    Ok(privacy_of_many(core, &[user_id]).await?.remove(&user_id).unwrap_or_else(default_privacy))
}

/// [`privacy_of`] for many users at once, keyed by user ID (unknown users are absent).
pub async fn privacy_of_many(
    core: &Core,
    user_ids: &[Uuid],
) -> AppResult<std::collections::HashMap<Uuid, PrivacySettings>> {
    let rows: Vec<(Uuid, serde_json::Value)> =
        sqlx::query_as("SELECT id, privacy FROM users WHERE id = ANY($1)").bind(user_ids).fetch_all(&core.db).await?;
    let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
    let minors = crate::age::minors_among(core, &ids).await?;
    Ok(rows.into_iter().map(|(id, value)| (id, privacy_as_applied(&value, minors.contains(&id)))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_json_yields_defaults() {
        let p = privacy_from_json(&serde_json::json!({}));
        assert!(findable_by_anyone(&p) && p.record_history && p.share_battery);
        assert_eq!(appearance_from_json(&serde_json::Value::Null).dopamine_level, 3);
    }

    #[test]
    fn stored_settings_roundtrip_through_json() {
        let mut p = default_privacy();
        p.discoverable_to = Audience::AUDIENCE_NOBODY.into();
        p.location_history_days = 30;
        let json = serde_json::to_value(&p).unwrap();
        let back = privacy_from_json(&json);
        assert!(!findable_by_friends_of_friends(&back));
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

    #[test]
    fn settings_from_before_discoverable_to_keep_their_switch() {
        let on = privacy_from_json(&serde_json::json!({ "discoverable": true, "recordHistory": true }));
        assert_eq!(on.discoverable_to, Audience::AUDIENCE_EVERYONE);
        // Proto JSON leaves a switch that is off out.
        let off = privacy_from_json(&serde_json::json!({ "recordHistory": true }));
        assert_eq!(off.discoverable_to, Audience::AUDIENCE_NOBODY);
        assert!(off.record_history, "the rest of the stored settings stays");
        let stored = serde_json::to_value(minor_defaults(default_privacy())).unwrap();
        assert!(stored.get("discoverableTo").is_some(), "the JSON name the shim looks for: {stored}");
        assert_eq!(privacy_from_json(&stored).discoverable_to, Audience::AUDIENCE_FRIENDS_OF_FRIENDS);
    }

    #[test]
    fn minors_reach_friends_of_friends_at_most() {
        let fof = Audience::AUDIENCE_FRIENDS_OF_FRIENDS;
        let limited = within_minor_limits(default_privacy());
        assert_eq!((limited.discoverable_to, limited.friend_requests_from), (fof.into(), fof.into()));
        assert_eq!(
            limited.default_note_visibility,
            Visibility::VISIBILITY_PUBLIC,
            "only the two audiences are limited"
        );
        let mut narrow = default_privacy();
        narrow.friend_requests_from = Audience::AUDIENCE_NOBODY.into();
        narrow.discoverable_to = Audience::AUDIENCE_NOBODY.into();
        assert_eq!(
            within_minor_limits(narrow.clone()).friend_requests_from,
            Audience::AUDIENCE_NOBODY,
            "narrower stays"
        );
        assert!(check_minor_limits(&narrow, true).is_ok());
        assert_eq!(check_minor_limits(&default_privacy(), true).unwrap_err().reason, "SETTING_RESTRICTED");
        assert!(check_minor_limits(&default_privacy(), false).is_ok());
        assert!(
            check_minor_limits(&minor_defaults(default_privacy()), true).is_ok(),
            "the defaults are within the limits"
        );
    }
}
