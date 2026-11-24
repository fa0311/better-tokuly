//! `PreferencesService`: category toggles, quiet hours and the push pause.
use chrono::Utc;
use tokuly_proto::notify::*;
use tokuly_testkit::TestUser;
use uuid::Uuid;

use crate::{
    common::*,
    mutes::{LIST_MUTED, MUTE, UNMUTE},
};

const GET: &str = "tokuly.notify.v1.PreferencesService/GetPreferences";
const UPDATE: &str = "tokuly.notify.v1.PreferencesService/UpdatePreferences";
const PAUSE: &str = "tokuly.notify.v1.PreferencesService/PausePush";
const RESUME: &str = "tokuly.notify.v1.PreferencesService/ResumePush";

async fn get(ctx: &Ctx, user: &TestUser) -> Preferences {
    let response: GetPreferencesResponse =
        ctx.app.call(GET, Some(user), &GetPreferencesRequest::default()).await.unwrap();
    response.preferences.into_option().unwrap()
}

async fn update(ctx: &Ctx, user: &TestUser, request: UpdatePreferencesRequest) -> Result<Preferences, String> {
    match ctx.app.call::<_, UpdatePreferencesResponse>(UPDATE, Some(user), &request).await {
        Ok(response) => Ok(response.preferences.into_option().unwrap()),
        Err(e) => Err(e.reason),
    }
}

fn change(category: &str, inbox: Option<bool>, push: Option<bool>) -> CategoryChange {
    CategoryChange { category: category.into(), inbox, push, ..Default::default() }
}

fn category<'a>(preferences: &'a Preferences, id: &str) -> &'a CategoryPreference {
    preferences.categories.iter().find(|c| c.category == id).unwrap_or_else(|| panic!("category {id} is listed"))
}

fn quiet(enabled: bool, start_minute: i32, end_minute: i32) -> QuietHours {
    QuietHours { enabled, start_minute, end_minute, ..Default::default() }
}

#[tokio::test]
async fn every_preferences_rpc_requires_sign_in() {
    let ctx = start().await;
    let app = &ctx.app;
    let someone = Uuid::now_v7().to_string();
    let errors = [
        app.call::<_, GetPreferencesResponse>(GET, None, &GetPreferencesRequest::default()).await.unwrap_err(),
        app.call::<_, UpdatePreferencesResponse>(UPDATE, None, &UpdatePreferencesRequest::default()).await.unwrap_err(),
        app.call::<_, PausePushResponse>(PAUSE, None, &PausePushRequest::default()).await.unwrap_err(),
        app.call::<_, ResumePushResponse>(RESUME, None, &ResumePushRequest::default()).await.unwrap_err(),
        app.call::<_, MuteUserResponse>(
            MUTE,
            None,
            &MuteUserRequest { user_id: someone.clone(), ..Default::default() },
        )
        .await
        .unwrap_err(),
        app.call::<_, UnmuteUserResponse>(UNMUTE, None, &UnmuteUserRequest { user_id: someone, ..Default::default() })
            .await
            .unwrap_err(),
        app.call::<_, ListMutedUsersResponse>(LIST_MUTED, None, &ListMutedUsersRequest::default()).await.unwrap_err(),
    ];
    for err in errors {
        assert_eq!((err.status, err.reason.as_str()), (401, "SIGN_IN_REQUIRED"));
    }
}

#[tokio::test]
async fn defaults_are_everything_on() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let preferences = get(&ctx, &alice).await;

    let ids: Vec<&str> = preferences.categories.iter().map(|c| c.category.as_str()).collect();
    assert_eq!(
        ids,
        [
            "social",
            "chat",
            "geo",
            "notes",
            "video",
            "live",
            "spaces",
            "shorts",
            "engagement",
            "stamp",
            "wallet",
            "ask",
            "plans",
            "school",
            "community",
            "quest",
            "feed",
            "system"
        ]
    );
    assert!(preferences.categories.iter().all(|c| c.inbox && c.push), "everything is on by default");
    assert!(preferences.categories.iter().all(|c| !c.label.is_empty() && !c.description.is_empty()));
    assert_eq!(category(&preferences, "chat").label, "チャット・通話", "labels are in the account's language");
    assert!(category(&preferences, "system").inbox_locked && !category(&preferences, "chat").inbox_locked);

    let quiet_hours = preferences.quiet_hours.as_option().unwrap();
    assert_eq!(
        (quiet_hours.enabled, quiet_hours.start_minute, quiet_hours.end_minute),
        (false, 1320, 420),
        "off, with 22:00-07:00 as the suggestion"
    );
    assert!(!preferences.quiet_now);
    assert!(!preferences.push_pause.as_option().map(|p| p.paused).unwrap_or_default());
    assert_eq!(preferences.time_zone, "Asia/Tokyo");

    sqlx::query("UPDATE users SET locale = 'en' WHERE id = $1").bind(alice.id).execute(&ctx.app.core.db).await.unwrap();
    assert_eq!(category(&get(&ctx, &alice).await, "chat").label, "Chat & calls");
}

#[tokio::test]
async fn category_toggles_change_independently_and_per_user() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);

    let preferences = update(
        &ctx,
        &alice,
        UpdatePreferencesRequest {
            categories: vec![change("chat", None, Some(false)), change("live", Some(false), None)],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!((category(&preferences, "chat").inbox, category(&preferences, "chat").push), (true, false));
    assert_eq!((category(&preferences, "live").inbox, category(&preferences, "live").push), (false, true));
    assert!(
        category(&preferences, "video").inbox && category(&preferences, "video").push,
        "unmentioned categories keep their value"
    );

    // A later partial change keeps the other toggle.
    let preferences = update(
        &ctx,
        &alice,
        UpdatePreferencesRequest { categories: vec![change("chat", Some(false), None)], ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!((category(&preferences, "chat").inbox, category(&preferences, "chat").push), (false, false));
    let preferences = update(
        &ctx,
        &alice,
        UpdatePreferencesRequest { categories: vec![change("chat", Some(true), Some(true))], ..Default::default() },
    )
    .await
    .unwrap();
    assert!(category(&preferences, "chat").inbox && category(&preferences, "chat").push);
    assert!(!category(&get(&ctx, &alice).await, "live").inbox, "changes persist");

    assert!(get(&ctx, &bob).await.categories.iter().all(|c| c.inbox && c.push), "preferences are per user");
}

#[tokio::test]
async fn category_changes_are_validated_before_anything_is_saved() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;

    // One bad entry rejects the whole request.
    let request = UpdatePreferencesRequest {
        categories: vec![change("chat", Some(false), Some(false)), change("nope", None, Some(false))],
        ..Default::default()
    };
    assert_eq!(update(&ctx, &alice, request).await.unwrap_err(), "CATEGORY_UNKNOWN");
    assert!(category(&get(&ctx, &alice).await, "chat").inbox, "nothing was applied");

    // Account and security notices cannot leave the inbox, but their push can be turned off.
    let request =
        UpdatePreferencesRequest { categories: vec![change("system", Some(false), None)], ..Default::default() };
    assert_eq!(update(&ctx, &alice, request).await.unwrap_err(), "CATEGORY_LOCKED");
    let preferences = update(
        &ctx,
        &alice,
        UpdatePreferencesRequest { categories: vec![change("system", Some(true), Some(false))], ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!((category(&preferences, "system").inbox, category(&preferences, "system").push), (true, false));
}

#[tokio::test]
async fn quiet_hours_are_saved_and_validated() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;

    let preferences = update(
        &ctx,
        &alice,
        UpdatePreferencesRequest { quiet_hours: quiet(true, 23 * 60, 6 * 60 + 30).into(), ..Default::default() },
    )
    .await
    .unwrap();
    let saved = preferences.quiet_hours.as_option().unwrap();
    assert_eq!((saved.enabled, saved.start_minute, saved.end_minute), (true, 1380, 390));

    // A whole-day-minus-a-minute window is certainly in effect; its opposite is not.
    let now_minute = {
        use chrono::Timelike as _;
        let local = Utc::now().with_timezone(&chrono_tz::Asia::Tokyo);
        (local.hour() * 60 + local.minute()) as i32
    };
    let after_now = (now_minute + 5) % 1440;
    let preferences = update(
        &ctx,
        &alice,
        UpdatePreferencesRequest {
            quiet_hours: quiet(true, (after_now + 1) % 1440, after_now).into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(preferences.quiet_now);
    let preferences = update(
        &ctx,
        &alice,
        UpdatePreferencesRequest {
            quiet_hours: quiet(false, (after_now + 1) % 1440, after_now).into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(!preferences.quiet_now, "a disabled window is never in effect");
    assert_eq!(preferences.quiet_hours.as_option().unwrap().end_minute, after_now, "but its times are kept");

    // Updating categories alone leaves quiet hours untouched.
    let preferences = update(
        &ctx,
        &alice,
        UpdatePreferencesRequest { categories: vec![change("chat", None, Some(false))], ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(preferences.quiet_hours.as_option().unwrap().end_minute, after_now);

    for bad in [quiet(true, -1, 100), quiet(true, 100, 1440), quiet(true, 600, 600)] {
        assert_eq!(
            update(&ctx, &alice, UpdatePreferencesRequest { quiet_hours: bad.into(), ..Default::default() })
                .await
                .unwrap_err(),
            "QUIET_HOURS_INVALID"
        );
    }
}

#[tokio::test]
async fn push_can_be_paused_for_a_while_or_until_resumed() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let pause = |seconds: i32| {
        let (app, alice) = (&ctx.app, &alice);
        async move {
            app.call::<_, PausePushResponse>(
                PAUSE,
                Some(alice),
                &PausePushRequest { duration_seconds: seconds, ..Default::default() },
            )
            .await
        }
    };

    // Timed.
    let preferences = pause(3600).await.unwrap().preferences.into_option().unwrap();
    let state = preferences.push_pause.as_option().unwrap();
    assert!(state.paused);
    let until = state.until.as_option().expect("a timed pause says when it ends").seconds;
    let expected = Utc::now().timestamp() + 3600;
    assert!((until - expected).abs() <= 5, "{until} vs {expected}");

    // Until resumed.
    let preferences = pause(0).await.unwrap().preferences.into_option().unwrap();
    let state = preferences.push_pause.as_option().unwrap();
    assert!(state.paused && state.until.as_option().is_none());
    assert!(get(&ctx, &alice).await.push_pause.as_option().unwrap().paused);

    let resumed: ResumePushResponse = ctx.app.call(RESUME, Some(&alice), &ResumePushRequest::default()).await.unwrap();
    assert!(!resumed.preferences.as_option().unwrap().push_pause.as_option().map(|p| p.paused).unwrap_or_default());
    // Resuming when nothing is paused (or nothing was ever saved) is fine.
    let bob = ctx.app.user("bob").await;
    let _: ResumePushResponse = ctx.app.call(RESUME, Some(&bob), &ResumePushRequest::default()).await.unwrap();

    // A pause that ran out reads as not paused.
    pause(60).await.unwrap();
    sqlx::query("UPDATE notify_settings SET push_paused_until = now() - interval '1 second' WHERE user_id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let state = get(&ctx, &alice).await.push_pause;
    assert!(!state.as_option().map(|p| p.paused).unwrap_or_default());
    assert!(state.as_option().and_then(|p| p.until.as_option()).is_none());

    for bad in [-1, 30 * 24 * 3600 + 1] {
        assert_eq!(pause(bad).await.unwrap_err().reason, "PAUSE_DURATION_INVALID");
    }
}
