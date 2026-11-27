//! Accounts under 18: the defaults they start with, the limits while they are
//! minors, and accounts that give their birthdate after sign-up.
use chrono::{Months, NaiveDate, Utc};
use tokuly_identity::maintenance::BirthdateGiven;
use tokuly_proto::{common::Visibility, identity::*};
use tokuly_testkit::{RpcError, TestApp, TestUser};

const MIGRATIONS: &[&str] = &["0001", "0100", "0400"];

async fn app() -> TestApp {
    TestApp::start(MIGRATIONS, |_core, identity, router| identity.register(router)).await
}

fn unique(prefix: &str) -> String {
    format!("{prefix}{}", tokuly_core::ids::random_base62(8).to_ascii_lowercase())
}

/// The birthdate of someone who turns `years` today.
fn born(years: u32) -> NaiveDate {
    Utc::now().date_naive().checked_sub_months(Months::new(12 * years)).unwrap()
}

async fn sign_up(app: &TestApp, birthdate: NaiveDate) -> TestUser {
    let tag = unique("age");
    let request = SignUpRequest {
        email: format!("{tag}@example.com"),
        password: "correct horse battery".into(),
        handle: tag,
        display_name: "テスト".into(),
        birthdate: birthdate.to_string(),
        accept_terms: true,
        ..Default::default()
    };
    let auth: AuthResponse = app.call("tokuly.identity.v1.AuthService/SignUp", None, &request).await.unwrap();
    let id = auth.me.as_option().unwrap().user.as_option().unwrap().id.parse().unwrap();
    app.sign_in(id).await
}

/// A user made before sign-up asked for a birthdate.
async fn without_birthdate(app: &TestApp) -> TestUser {
    let user = app.user("old").await;
    sqlx::query("UPDATE users SET birthdate = NULL WHERE id = $1").bind(user.id).execute(&app.core.db).await.unwrap();
    user
}

async fn privacy(app: &TestApp, user: &TestUser) -> PrivacySettings {
    let response: GetSettingsResponse = app
        .call("tokuly.identity.v1.AccountService/GetSettings", Some(user), &GetSettingsRequest::default())
        .await
        .unwrap();
    response.settings.into_option().unwrap().privacy.into_option().unwrap()
}

async fn me(app: &TestApp, user: &TestUser) -> Me {
    let response: GetMeResponse =
        app.call("tokuly.identity.v1.AccountService/GetMe", Some(user), &GetMeRequest::default()).await.unwrap();
    response.me.into_option().unwrap()
}

async fn update_privacy(app: &TestApp, user: &TestUser, privacy: PrivacySettings) -> Result<PrivacySettings, RpcError> {
    let request = UpdateSettingsRequest { privacy: privacy.into(), ..Default::default() };
    let response: UpdateSettingsResponse =
        app.call("tokuly.identity.v1.AccountService/UpdateSettings", Some(user), &request).await?;
    Ok(response.settings.into_option().unwrap().privacy.into_option().unwrap())
}

async fn set_birthdate(app: &TestApp, user: &TestUser, birthdate: &str) -> Result<SetBirthdateResponse, RpcError> {
    let request = SetBirthdateRequest { birthdate: birthdate.into(), ..Default::default() };
    app.call("tokuly.identity.v1.AccountService/SetBirthdate", Some(user), &request).await
}

const FOF: Audience = Audience::AUDIENCE_FRIENDS_OF_FRIENDS;

#[tokio::test]
async fn an_account_signed_up_under_18_starts_with_the_defaults_for_minors() {
    let app = app().await;
    for years in [13, 15, 17] {
        let teen = sign_up(&app, born(years)).await;
        let p = privacy(&app, &teen).await;
        assert_eq!(p.discoverable_to, FOF, "{years}: found by friends of friends only, no address books");
        assert_eq!(p.friend_requests_from, FOF, "{years}");
        assert_eq!(p.default_note_visibility, Visibility::VISIBILITY_FOLLOWERS, "{years}");
        assert!(!p.nearby_discoverable, "{years}: off until they turn it on");
        assert!(p.share_battery && p.record_history, "{years}: the other defaults stay");
        assert!(me(&app, &teen).await.minor);
        let created = app.events_by("identity.user.created", teen.id).await;
        assert_eq!(created[0].data["minor"], true);
    }
    for years in [18, 30] {
        let adult = sign_up(&app, born(years)).await;
        let p = privacy(&app, &adult).await;
        assert_eq!(p.discoverable_to, Audience::AUDIENCE_EVERYONE, "{years}");
        assert_eq!(p.friend_requests_from, Audience::AUDIENCE_EVERYONE, "{years}");
        assert_eq!(p.default_note_visibility, Visibility::VISIBILITY_PUBLIC, "{years}");
        assert!(p.nearby_discoverable, "{years}");
        assert!(!me(&app, &adult).await.minor);
        assert_eq!(app.events_by("identity.user.created", adult.id).await[0].data["minor"], false);
    }
}

#[tokio::test]
async fn the_defaults_stay_when_a_minor_comes_of_age() {
    let app = app().await;
    let teen = sign_up(&app, born(17)).await;
    sqlx::query("UPDATE users SET birthdate = $2 WHERE id = $1")
        .bind(teen.id)
        .bind(born(19))
        .execute(&app.core.db)
        .await
        .unwrap();
    let p = privacy(&app, &teen).await;
    assert!(!me(&app, &teen).await.minor);
    assert_eq!((p.discoverable_to, p.default_note_visibility), (FOF.into(), Visibility::VISIBILITY_FOLLOWERS.into()));
    // ...and now they may open them up.
    let opened =
        update_privacy(&app, &teen, PrivacySettings { discoverable_to: Audience::AUDIENCE_EVERYONE.into(), ..p })
            .await
            .unwrap();
    assert_eq!(opened.discoverable_to, Audience::AUDIENCE_EVERYONE);
}

#[tokio::test]
async fn a_minor_changes_the_defaults_but_reaches_friends_of_friends_at_most() {
    let app = app().await;
    let teen = sign_up(&app, born(15)).await;
    let p = privacy(&app, &teen).await;

    let narrower = PrivacySettings {
        discoverable_to: Audience::AUDIENCE_NOBODY.into(),
        friend_requests_from: Audience::AUDIENCE_FRIENDS.into(),
        default_note_visibility: Visibility::VISIBILITY_PUBLIC.into(),
        nearby_discoverable: true,
        ..p.clone()
    };
    let saved = update_privacy(&app, &teen, narrower).await.unwrap();
    assert_eq!(saved.discoverable_to, Audience::AUDIENCE_NOBODY);
    assert_eq!(saved.default_note_visibility, Visibility::VISIBILITY_PUBLIC, "the note default is theirs to change");
    assert!(saved.nearby_discoverable);

    for wider in [
        PrivacySettings { discoverable_to: Audience::AUDIENCE_EVERYONE.into(), ..p.clone() },
        PrivacySettings { friend_requests_from: Audience::AUDIENCE_EVERYONE.into(), ..p.clone() },
    ] {
        let err = update_privacy(&app, &teen, wider).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("invalid_argument", "SETTING_RESTRICTED"));
    }
    assert_eq!(privacy(&app, &teen).await.discoverable_to, Audience::AUDIENCE_NOBODY, "nothing changed");
}

#[tokio::test]
async fn what_a_minor_stored_before_the_limits_reads_within_them() {
    let app = app().await;
    let teen = app.user("teen").await;
    // Signed up before the defaults for minors: the defaults for everyone.
    store_privacy_from_before(&app, &teen, |_| {}).await;
    sqlx::query("UPDATE users SET birthdate = $2 WHERE id = $1")
        .bind(teen.id)
        .bind(born(16))
        .execute(&app.core.db)
        .await
        .unwrap();
    let p = privacy(&app, &teen).await;
    assert_eq!((p.discoverable_to, p.friend_requests_from), (FOF.into(), FOF.into()));
    let read = tokuly_identity::settings::privacy_of(&app.core, teen.id).await.unwrap();
    assert_eq!((read.discoverable_to, read.friend_requests_from), (FOF.into(), FOF.into()), "for every domain");
    assert!(!tokuly_identity::settings::findable_by_anyone(&read));
    // The screen sends back what it was shown; that is within the limits.
    assert!(update_privacy(&app, &teen, p).await.is_ok());
}

/// Stores settings as they were saved before `discoverable_to` (with the
/// switch `discoverable` on), changed by `edit`.
async fn store_privacy_from_before(app: &TestApp, user: &TestUser, edit: impl FnOnce(&mut PrivacySettings)) {
    let mut p = tokuly_identity::settings::default_privacy();
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

#[tokio::test]
async fn the_note_default_takes_followers() {
    let app = app().await;
    let adult = app.user("adult").await;
    let p = privacy(&app, &adult).await;
    let saved = update_privacy(
        &app,
        &adult,
        PrivacySettings { default_note_visibility: Visibility::VISIBILITY_FOLLOWERS.into(), ..p },
    )
    .await
    .unwrap();
    assert_eq!(saved.default_note_visibility, Visibility::VISIBILITY_FOLLOWERS);
}

#[tokio::test]
async fn a_client_that_does_not_know_discoverable_to_keeps_it() {
    let app = app().await;
    let user = app.user("old_client").await;
    let p = privacy(&app, &user).await;
    update_privacy(&app, &user, PrivacySettings { discoverable_to: Audience::AUDIENCE_NOBODY.into(), ..p.clone() })
        .await
        .unwrap();
    let saved = update_privacy(
        &app,
        &user,
        PrivacySettings { discoverable_to: Audience::AUDIENCE_UNSPECIFIED.into(), share_battery: false, ..p },
    )
    .await
    .unwrap();
    assert_eq!(saved.discoverable_to, Audience::AUDIENCE_NOBODY);
    assert!(!saved.share_battery);
}

#[tokio::test]
async fn an_account_without_a_birthdate_counts_as_a_minor_until_it_gives_one() {
    let app = app().await;
    let old = without_birthdate(&app).await;
    let before = me(&app, &old).await;
    assert!(before.birthdate.is_empty() && before.minor, "the client asks, and the limits apply meanwhile");
    let p = privacy(&app, &old).await;
    assert_eq!((p.discoverable_to, p.friend_requests_from), (FOF.into(), FOF.into()));
    assert_eq!(
        p.default_note_visibility,
        Visibility::VISIBILITY_PUBLIC,
        "defaults are decided when the birthdate is given"
    );
    assert!(tokuly_identity::age::is_minor(&app.core, old.id).await.unwrap());
}

#[tokio::test]
async fn a_minor_giving_the_birthdate_gets_the_defaults_for_minors_then() {
    let app = app().await;
    let old = without_birthdate(&app).await;
    // Settings chosen while the account had the defaults for adults.
    let mut chosen = privacy(&app, &old).await;
    chosen.share_battery = false;
    chosen.discoverable_to = FOF.into();
    chosen.birthday_visible_to = Audience::AUDIENCE_EVERYONE.into();
    chosen.birthday_show_year = true;
    update_privacy(&app, &old, chosen).await.unwrap();

    let birthdate = born(14);
    let response = set_birthdate(&app, &old, &birthdate.to_string()).await.unwrap();
    let me = response.me.as_option().unwrap();
    assert_eq!(me.birthdate, birthdate.to_string());
    assert!(me.minor);
    let p = response.settings.as_option().unwrap().privacy.as_option().unwrap().clone();
    assert_eq!((p.discoverable_to, p.friend_requests_from), (FOF.into(), FOF.into()));
    assert_eq!(p.default_note_visibility, Visibility::VISIBILITY_FOLLOWERS);
    assert!(!p.nearby_discoverable);
    assert!(!p.birthday_show_year, "the year of a minor is not shown to everyone");
    assert!(!p.share_battery, "their own choices stay");
    assert_eq!(privacy(&app, &old).await, p, "stored");

    let given = app.events_by("identity.user.birthdate_given", old.id).await;
    assert_eq!(given.len(), 1);
    let given: BirthdateGiven = given[0].data().unwrap();
    assert!(given.minor && given.user_id == old.id);
    assert_eq!(
        app.events_by("identity.user.privacy_changed", old.id).await.len(),
        2,
        "the change above, then the defaults"
    );

    let again = set_birthdate(&app, &old, "2000-01-01").await.unwrap_err();
    assert_eq!((again.code.as_str(), again.reason.as_str()), ("failed_precondition", "BIRTHDATE_ALREADY_SET"));
    assert_eq!(me_birthdate(&app, &old).await, birthdate.to_string(), "it cannot be changed");
}

/// The defaults for minors only narrow: what the account already keeps
/// tighter than them stays.
#[tokio::test]
async fn a_minor_giving_the_birthdate_keeps_what_was_narrower_than_the_defaults() {
    let app = app().await;
    let old = without_birthdate(&app).await;
    let mut chosen = privacy(&app, &old).await;
    chosen.discoverable_to = Audience::AUDIENCE_NOBODY.into();
    chosen.friend_requests_from = Audience::AUDIENCE_NOBODY.into();
    chosen.default_note_visibility = Visibility::VISIBILITY_FRIENDS.into();
    update_privacy(&app, &old, chosen).await.unwrap();

    let response = set_birthdate(&app, &old, &born(14).to_string()).await.unwrap();
    let p = response.settings.as_option().unwrap().privacy.as_option().unwrap().clone();
    assert_eq!(p.discoverable_to, Audience::AUDIENCE_NOBODY, "found by nobody stays");
    assert_eq!(p.friend_requests_from, Audience::AUDIENCE_NOBODY, "requests from nobody stay");
    assert_eq!(p.default_note_visibility, Visibility::VISIBILITY_FRIENDS, "notes for friends stay");
    assert!(!p.nearby_discoverable);
}

async fn me_birthdate(app: &TestApp, user: &TestUser) -> String {
    me(app, user).await.birthdate
}

#[tokio::test]
async fn an_adult_giving_the_birthdate_keeps_the_settings() {
    let app = app().await;
    let old = without_birthdate(&app).await;
    store_privacy_from_before(&app, &old, |p| p.share_battery = false).await;

    let response = set_birthdate(&app, &old, &born(25).to_string()).await.unwrap();
    assert!(!response.me.as_option().unwrap().minor);
    let p = response.settings.as_option().unwrap().privacy.as_option().unwrap().clone();
    assert_eq!(
        (p.discoverable_to, p.friend_requests_from),
        (Audience::AUDIENCE_EVERYONE.into(), Audience::AUDIENCE_EVERYONE.into()),
        "the limits are lifted"
    );
    assert!(!p.share_battery);
    let given: BirthdateGiven = app.events_by("identity.user.birthdate_given", old.id).await[0].data().unwrap();
    assert!(!given.minor);
    assert!(app.events_by("identity.user.privacy_changed", old.id).await.is_empty(), "nothing changed");
}

#[tokio::test]
async fn set_birthdate_validates_and_stores_nothing_under_the_minimum_age() {
    let app = app().await;
    let old = without_birthdate(&app).await;
    for (birthdate, reason) in [("soon", "BIRTHDATE_INVALID"), ("2999-01-01", "BIRTHDATE_INVALID")] {
        assert_eq!(set_birthdate(&app, &old, birthdate).await.unwrap_err().reason, reason, "{birthdate}");
    }
    let err = set_birthdate(&app, &old, &born(12).to_string()).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("permission_denied", "AGE_RESTRICTED"));
    assert!(me_birthdate(&app, &old).await.is_empty());
    assert!(app.events_by("identity.user.birthdate_given", old.id).await.is_empty());
    // A signed-up account has one already.
    let adult = app.user("adult").await;
    assert_eq!(set_birthdate(&app, &adult, "2001-01-01").await.unwrap_err().reason, "BIRTHDATE_ALREADY_SET");
    let anonymous = app
        .call::<_, SetBirthdateResponse>(
            "tokuly.identity.v1.AccountService/SetBirthdate",
            None,
            &SetBirthdateRequest::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(anonymous.code, "unauthenticated");
}

#[tokio::test]
async fn two_answers_at_once_store_one_birthdate() {
    let app = app().await;
    let old = without_birthdate(&app).await;
    let (minor, adult) = (born(15).to_string(), born(30).to_string());
    let (a, b) = tokio::join!(set_birthdate(&app, &old, &minor), set_birthdate(&app, &old, &adult));
    assert_eq!([a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(), 1, "{:?} {:?}", a.err(), b.err());
    assert_eq!(app.events_by("identity.user.birthdate_given", old.id).await.len(), 1);
    let stored = me_birthdate(&app, &old).await;
    assert!(stored == minor || stored == adult);
}

#[tokio::test]
async fn a_bot_goes_by_its_owner() {
    let app = app().await;
    let (teen, adult) = (app.user("teen").await, app.user("adult").await);
    sqlx::query("UPDATE users SET birthdate = $2 WHERE id = $1")
        .bind(teen.id)
        .bind(born(15))
        .execute(&app.core.db)
        .await
        .unwrap();
    let mut bots = Vec::new();
    for owner in [&teen, &adult] {
        let mut tx = app.core.db.begin().await.unwrap();
        let bot =
            tokuly_identity::bots::create_bot_account(&mut tx, owner.id, &unique("bot"), "ボット", "").await.unwrap();
        tx.commit().await.unwrap();
        bots.push(bot);
    }
    let minors = tokuly_identity::age::minors_among(&app.core, &[bots[0], bots[1], teen.id, adult.id]).await.unwrap();
    assert_eq!(minors, [bots[0], teen.id].into_iter().collect());
}
