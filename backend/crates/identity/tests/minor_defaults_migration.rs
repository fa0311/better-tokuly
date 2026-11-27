//! Migration 0101: the defaults for minors for the accounts under 18 that
//! gave their birthdate before those defaults came. A test binary of its own:
//! the migration touches every account of the database.
use chrono::{Months, NaiveDate, Utc};
use tokuly_identity::{
    maintenance::BirthdateGiven,
    settings::{default_privacy, minor_defaults, privacy_from_json},
};
use tokuly_proto::{common::Visibility, identity::*};
use tokuly_testkit::{TestApp, TestUser};

const FOF: Audience = Audience::AUDIENCE_FRIENDS_OF_FRIENDS;

/// The birthdate of someone who turns `years` today.
fn born(years: u32) -> NaiveDate {
    Utc::now().date_naive().checked_sub_months(Months::new(12 * years)).unwrap()
}

async fn set_birthdate(app: &TestApp, user: &TestUser, birthdate: Option<NaiveDate>) {
    sqlx::query("UPDATE users SET birthdate = $2 WHERE id = $1")
        .bind(user.id)
        .bind(birthdate)
        .execute(&app.core.db)
        .await
        .unwrap();
}

async fn stored(app: &TestApp, user: &TestUser) -> PrivacySettings {
    let value: serde_json::Value =
        sqlx::query_scalar("SELECT privacy FROM users WHERE id = $1").bind(user.id).fetch_one(&app.core.db).await.unwrap();
    privacy_from_json(&value)
}

/// Settings as saved before `discoverable_to` (the switch `discoverable` on),
/// changed by `edit`.
async fn store_privacy_from_before(app: &TestApp, user: &TestUser, edit: impl FnOnce(&mut PrivacySettings)) {
    let mut p = default_privacy();
    edit(&mut p);
    let mut json = serde_json::to_value(&p).unwrap();
    let fields = json.as_object_mut().unwrap();
    fields.remove("discoverableTo");
    fields.insert("discoverable".into(), true.into());
    sqlx::query("UPDATE users SET privacy = $2 WHERE id = $1")
        .bind(user.id)
        .bind(json)
        .execute(&app.core.db)
        .await
        .unwrap();
}

/// Accounts under 18 that gave their birthdate before the defaults for
/// minors came get them once (migration 0101), narrowing only; the other
/// domains hear `identity.user.birthdate_given`.
#[tokio::test]
async fn minors_from_before_the_defaults_get_them_once() {
    let app = TestApp::start(&["0001", "0100", "0400"], |_core, identity, router| identity.register(router)).await;
    let untouched = app.user("teen").await;
    let chose = app.user("teen").await;
    store_privacy_from_before(&app, &chose, |p| {
        p.default_note_visibility = Visibility::VISIBILITY_FRIENDS.into();
        p.friend_requests_from = Audience::AUDIENCE_NOBODY.into();
        p.share_battery = false;
    })
    .await;
    for teen in [&untouched, &chose] {
        set_birthdate(&app, teen, Some(born(16))).await;
    }
    let adult = app.user("adult").await;
    let old = app.user("old").await;
    set_birthdate(&app, &old, None).await;

    sqlx::raw_sql(include_str!("../../../migrations/0101_identity_minor_defaults.sql"))
        .execute(&app.core.db)
        .await
        .unwrap();

    assert_eq!(stored(&app, &untouched).await, minor_defaults(default_privacy()), "as a minor signing up today");
    let p = stored(&app, &chose).await;
    assert_eq!((p.discoverable_to, p.friend_requests_from), (FOF.into(), Audience::AUDIENCE_NOBODY.into()));
    assert_eq!(p.default_note_visibility, Visibility::VISIBILITY_FRIENDS, "narrower stays");
    assert!(!p.nearby_discoverable && !p.share_battery);
    assert_eq!(stored(&app, &adult).await, default_privacy());

    for teen in [&untouched, &chose] {
        let given: BirthdateGiven = app.events_by("identity.user.birthdate_given", teen.id).await[0].data().unwrap();
        assert!(given.minor);
        assert_eq!(app.events_by("identity.user.privacy_changed", teen.id).await.len(), 1);
    }
    assert!(app.events_by("identity.user.birthdate_given", adult.id).await.is_empty());
    assert!(app.events_by("identity.user.privacy_changed", adult.id).await.is_empty());
    assert_eq!(
        app.events_by("identity.user.privacy_changed", old.id).await.len(),
        1,
        "counted as a minor: search reads the settings again"
    );
}
