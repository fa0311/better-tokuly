//! Badges: thresholds, the catalogue, secrets, the public award API,
//! showcase and title, rarity, and awarding by hand.
use serde_json::json;
use tokuly_core::prelude::*;
use tokuly_proto::{
    identity::{
        GetProfileRequest as IdentityProfileRequest, GetProfileResponse as IdentityProfileResponse, get_profile_request,
    },
    quest::{quest_event::Event as Push, *},
};
use tokuly_quest::badges::{self, catalog};
use tokuly_testkit::TestUser;

use crate::common::*;

async fn catalog(ctx: &Ctx, viewer: Option<&TestUser>) -> ListBadgeCatalogResponse {
    ctx.call(BADGES, "ListBadgeCatalog", viewer, &ListBadgeCatalogRequest::default()).await.unwrap()
}

fn find<'a>(catalog: &'a ListBadgeCatalogResponse, key: &str) -> &'a Badge {
    catalog
        .families
        .iter()
        .flat_map(|family| family.badges.iter())
        .find(|badge| badge.key == key)
        .unwrap_or_else(|| panic!("badge {key} is not in the catalogue"))
}

async fn set_showcase(
    ctx: &Ctx,
    user: &TestUser,
    keys: &[&str],
) -> Result<SetShowcaseResponse, tokuly_testkit::RpcError> {
    ctx.call(
        BADGES,
        "SetShowcase",
        Some(user),
        &SetShowcaseRequest { badge_keys: keys.iter().map(|key| key.to_string()).collect(), ..Default::default() },
    )
    .await
}

async fn set_title(ctx: &Ctx, user: &TestUser, key: &str) -> Result<SetTitleResponse, tokuly_testkit::RpcError> {
    ctx.call(BADGES, "SetTitle", Some(user), &SetTitleRequest { badge_key: key.to_owned(), ..Default::default() }).await
}

async fn user_badges(
    ctx: &Ctx,
    viewer: Option<&TestUser>,
    owner: &TestUser,
) -> Result<ListUserBadgesResponse, tokuly_testkit::RpcError> {
    ctx.call(
        BADGES,
        "ListUserBadges",
        viewer,
        &ListUserBadgesRequest { user_id: owner.id.to_string(), ..Default::default() },
    )
    .await
}

#[tokio::test]
async fn a_counted_badge_unlocks_when_its_threshold_is_crossed() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let mut listener = ctx.app.listen_user(alice.id).await;

    ctx.feed(event(
        "identity.user.created",
        Some(alice.id),
        json!({ "user_id": alice.id, "handle": alice.handle, "invite_code": "" }),
    ))
    .await;
    ctx.friend_accepted(&alice, &bob).await;
    ctx.flush(&[&alice]).await;

    let owned = ctx.badge_keys(&alice).await;
    assert!(owned.contains(&"welcome".to_owned()) && owned.contains(&"first_friend".to_owned()), "{owned:?}");
    assert!(!owned.contains(&"friends_circle_bronze".to_owned()), "five friends are not there yet");

    let unlocked: Vec<Badge> = pushes(&mut listener)
        .await
        .into_iter()
        .filter_map(|push| if let Push::BadgeUnlocked(badge) = push { Some(*badge) } else { None })
        .collect();
    let first_friend = unlocked.iter().find(|badge| badge.key == "first_friend").expect("the unlock is pushed");
    assert!(first_friend.owned);
    assert_eq!(
        (first_friend.family.as_str(), first_friend.name.as_option().unwrap().ja.as_str()),
        ("friends", "はじめての友達")
    );

    let notices = ctx.notifications(&alice, "quest.badge").await;
    assert!(notices.iter().any(|notice| notice["data"]["badge_key"] == "first_friend"
        && notice["title"]["ja"] == "バッジをゲット!"
        && notice["path"] == "/quest/badges/first_friend"));
    // What signing up brings is pushed to open clients but is no notification:
    // a new account's inbox stays empty until something happens.
    assert!(unlocked.iter().any(|badge| badge.key == "welcome"), "the welcome badge is pushed");
    assert!(notices.iter().all(|notice| notice["data"]["badge_key"] != "welcome"), "{notices:?}");

    // Four more friends: the bronze tier of the line, once.
    for _ in 0..4 {
        ctx.friend_accepted(&alice, &ctx.app.user("friend").await).await;
    }
    ctx.flush(&[&alice]).await;
    ctx.friend_accepted(&alice, &ctx.app.user("friend").await).await;
    ctx.flush(&[&alice]).await;
    let owned = ctx.badge_keys(&alice).await;
    assert_eq!(owned.iter().filter(|key| key.as_str() == "friends_circle_bronze").count(), 1);
    assert_eq!(
        ctx.notifications(&alice, "quest.badge")
            .await
            .iter()
            .filter(|notice| notice["data"]["badge_key"] == "friends_circle_bronze")
            .count(),
        1
    );
    assert!(
        ctx.notifications(&alice, "quest.badge")
            .await
            .iter()
            .any(|notice| notice["body"]["ja"] == "🤝 友達の輪（ブロンズ）")
    );

    // The catalogue shows what is owned and how far the next tier is.
    let catalog = catalog(&ctx, Some(&alice)).await;
    let bronze = find(&catalog, "friends_circle_bronze");
    assert!(bronze.owned && bronze.earned_at.as_option().is_some());
    assert_eq!((bronze.progress, bronze.target, bronze.tier.as_known()), (5, 5, Some(BadgeTier::BADGE_TIER_BRONZE)));
    let silver = find(&catalog, "friends_circle_silver");
    assert_eq!((silver.owned, silver.progress, silver.target, silver.line.as_str()), (false, 6, 25, "friends_circle"));
    assert_eq!(silver.description.as_option().unwrap().en, "Make 25 friends");
    assert!(catalog.owned_count >= 3);
}

#[tokio::test]
async fn the_catalogue_is_public_and_keeps_its_secrets() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;

    let public = catalog(&ctx, None).await;
    assert!(public.total_count >= 60, "only {} badges", public.total_count);
    assert_eq!(public.owned_count, 0);
    let families: Vec<&str> = public.families.iter().map(|family| family.key.as_str()).collect();
    for family in ["friends", "explore", "chat", "stamps", "creators", "study", "streaks", "seasons", "secret"] {
        assert!(families.contains(&family), "family {family} is missing");
    }
    assert_eq!(public.families.iter().map(|family| family.badges.len() as i32).sum::<i32>(), public.total_count);
    let gold = find(&public, "chatter_gold");
    assert_eq!(
        (gold.tier.as_known(), gold.target, gold.title.as_option().unwrap().ja.as_str()),
        (Some(BadgeTier::BADGE_TIER_GOLD), 10000, "おしゃべり番長")
    );

    // A secret badge says that it exists — not what it is, not even by its key.
    let stand_in = catalog::masked_key(catalog::find("early_bird").unwrap());
    let hidden = find(&public, &stand_in);
    assert!(hidden.secret && !hidden.owned);
    assert!(
        hidden.name.as_option().is_none()
            && hidden.description.as_option().is_none()
            && hidden.emoji.is_empty()
            && hidden.title.as_option().is_none()
    );
    assert!(!public.families.iter().flat_map(|family| family.badges.iter()).any(|badge| badge.key == "early_bird"));
    // It can be looked up by what the catalogue showed; guessing its name finds nothing.
    let response: GetBadgeResponse = ctx
        .call(BADGES, "GetBadge", Some(&alice), &GetBadgeRequest { key: stand_in.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(response.badge.as_option().unwrap().name.as_option().is_none());
    for viewer in [Some(&alice), None] {
        assert_error(
            ctx.call::<_, GetBadgeResponse>(
                BADGES,
                "GetBadge",
                viewer,
                &GetBadgeRequest { key: "early_bird".into(), ..Default::default() },
            )
            .await,
            "not_found",
            "BADGE_NOT_FOUND",
        );
    }

    // Once earned it shows itself to its owner, and to the friends looking at their badges.
    assert!(badges::award(&ctx.app.core, alice.id, "early_bird").await.unwrap());
    let revealed = catalog(&ctx, Some(&alice)).await;
    let mine = find(&revealed, "early_bird");
    assert!(mine.owned && mine.secret);
    assert_eq!((mine.name.as_option().unwrap().ja.as_str(), mine.emoji.as_str()), ("早起き", "🐓"));
    let by_name: GetBadgeResponse = ctx
        .call(BADGES, "GetBadge", Some(&alice), &GetBadgeRequest { key: "early_bird".into(), ..Default::default() })
        .await
        .unwrap();
    assert!(by_name.badge.as_option().unwrap().owned);
    let (friend, stranger) = (ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &friend).await;
    assert!(
        find(&catalog(&ctx, Some(&friend)).await, &stand_in).name.as_option().is_none(),
        "still a secret in the friend's own catalogue"
    );
    let seen = user_badges(&ctx, Some(&friend), &alice).await.unwrap();
    assert_eq!(
        seen.badges.iter().find(|badge| badge.key == "early_bird").unwrap().name.as_option().unwrap().en,
        "Early Bird"
    );
    // A stranger sees what Alice put on show, which is nothing so far.
    assert!(user_badges(&ctx, Some(&stranger), &alice).await.unwrap().badges.is_empty());
    assert!(user_badges(&ctx, None, &alice).await.unwrap().badges.is_empty());

    assert_error(
        ctx.call::<_, GetBadgeResponse>(
            BADGES,
            "GetBadge",
            None,
            &GetBadgeRequest { key: "no_such_badge".into(), ..Default::default() },
        )
        .await,
        "not_found",
        "BADGE_NOT_FOUND",
    );
}

#[tokio::test]
async fn other_domains_award_badges_through_the_public_function() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let mut listener = ctx.app.listen_user(alice.id).await;

    assert!(badges::award(&ctx.app.core, alice.id, "bug_hunter").await.unwrap(), "new: awarded");
    assert!(!badges::award(&ctx.app.core, alice.id, "bug_hunter").await.unwrap(), "already owned: not awarded again");
    assert!(ctx.badge_keys(&alice).await.contains(&"bug_hunter".to_owned()));
    assert_eq!(
        ctx.notifications(&alice, "quest.badge")
            .await
            .iter()
            .filter(|notice| notice["data"]["badge_key"] == "bug_hunter")
            .count(),
        1
    );
    let unlocked = pushes(&mut listener)
        .await
        .into_iter()
        .filter(|push| matches!(push, Push::BadgeUnlocked(badge) if badge.key == "bug_hunter"))
        .count();
    assert_eq!(unlocked, 1);

    // A badge with a title is news for the friends' feed; a plain one is not.
    badges::award(&ctx.app.core, alice.id, "event_winner").await.unwrap();
    let activities = ctx.app.events_by("activity.recorded", alice.id).await;
    assert_eq!(activities.len(), 1);
    assert_eq!(
        (
            activities[0].data["kind"].as_str(),
            activities[0].data["audience"].as_str(),
            activities[0].data["data"]["badge_key"].as_str()
        ),
        (Some("quest.badge"), Some("friends"), Some("bug_hunter"))
    );
    assert_eq!(activities[0].data["summary"]["ja"], "称号「バグハンター」をゲット!");

    let unknown_badge = badges::award(&ctx.app.core, alice.id, "no_such_badge").await.unwrap_err();
    assert_eq!(unknown_badge.reason, "BADGE_NOT_FOUND");
    let unknown_user = badges::award(&ctx.app.core, Uuid::now_v7(), "bug_hunter").await.unwrap_err();
    assert_eq!(unknown_user.reason, "USER_NOT_FOUND");
}

#[tokio::test]
async fn friends_who_own_a_badge_are_shown_with_it() {
    let ctx = start().await;
    let (alice, bob, carol, dave) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
    );
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    for owner in [&bob, &dave] {
        badges::award(&ctx.app.core, owner.id, "event_winner").await.unwrap();
    }
    let response: GetBadgeResponse = ctx
        .call(BADGES, "GetBadge", Some(&alice), &GetBadgeRequest { key: "event_winner".into(), ..Default::default() })
        .await
        .unwrap();
    let badge = response.badge.as_option().unwrap();
    assert!(!badge.owned);
    assert_eq!(badge.name.as_option().unwrap().en, "Event Winner");
    // Bob is a friend who has it; Dave has it but is nobody to Alice.
    assert_eq!(response.friend_owner_count, 1);
    assert_eq!(response.friend_owners.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), vec![bob.id.to_string()]);
}

#[tokio::test]
async fn the_showcase_and_title_are_chosen_from_owned_badges() {
    let ctx = start().await;
    let (alice, bob, stranger) =
        (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;
    for key in ["bug_hunter", "event_winner", "early_adopter", "welcome", "first_friend", "streak_3"] {
        badges::award(&ctx.app.core, alice.id, key).await.unwrap();
    }

    let shown = set_showcase(&ctx, &alice, &["early_adopter", "bug_hunter"]).await.unwrap();
    assert_eq!(
        shown.showcase.iter().map(|badge| badge.key.as_str()).collect::<Vec<_>>(),
        vec!["early_adopter", "bug_hunter"],
        "in the order chosen"
    );
    assert_error(
        set_showcase(
            &ctx,
            &alice,
            &["bug_hunter", "event_winner", "early_adopter", "welcome", "first_friend", "streak_3"],
        )
        .await,
        "invalid_argument",
        "TOO_MANY_BADGES",
    );
    assert_error(
        set_showcase(&ctx, &alice, &["bug_hunter", "bug_hunter"]).await,
        "invalid_argument",
        "DUPLICATE_BADGE",
    );
    assert_error(set_showcase(&ctx, &alice, &["chatter_gold"]).await, "failed_precondition", "BADGE_NOT_OWNED");
    assert_error(set_showcase(&ctx, &alice, &["no_such_badge"]).await, "failed_precondition", "BADGE_NOT_OWNED");
    assert_error(
        ctx.call::<_, SetShowcaseResponse>(BADGES, "SetShowcase", None, &SetShowcaseRequest::default()).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );

    let title = set_title(&ctx, &alice, "bug_hunter").await.unwrap();
    assert_eq!(title.title.as_option().unwrap().text.as_option().unwrap().ja, "バグハンター");
    assert_error(set_title(&ctx, &alice, "event_winner").await, "failed_precondition", "BADGE_HAS_NO_TITLE");
    assert_error(set_title(&ctx, &alice, "chatter_gold").await, "failed_precondition", "BADGE_NOT_OWNED");
    assert_error(
        ctx.call::<_, SetTitleResponse>(BADGES, "SetTitle", None, &SetTitleRequest::default()).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );

    // Everybody sees them: on the badge list, the quest card and the profile.
    let seen = user_badges(&ctx, Some(&bob), &alice).await.unwrap();
    assert_eq!(
        seen.showcase.iter().map(|badge| badge.key.as_str()).collect::<Vec<_>>(),
        vec!["early_adopter", "bug_hunter"]
    );
    assert_eq!(seen.title.as_option().unwrap().badge_key, "bug_hunter");
    assert_eq!(seen.badges.len(), 6, "a friend sees everything Alice earned");
    // Anybody else sees what she chose to show, and nothing of when she earned the rest.
    for viewer in [Some(&stranger), None] {
        let seen = user_badges(&ctx, viewer, &alice).await.unwrap();
        assert_eq!(
            seen.badges.iter().map(|badge| badge.key.as_str()).collect::<Vec<_>>(),
            vec!["early_adopter", "bug_hunter"]
        );
        assert_eq!(seen.showcase.len(), 2);
        assert_eq!(seen.title.as_option().unwrap().badge_key, "bug_hunter");
    }
    let card: GetProfileResponse = ctx
        .call(QUEST, "GetProfile", None, &GetProfileRequest { user_id: alice.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    let card = card.profile.as_option().unwrap();
    assert_eq!((card.level, card.badge_count, card.hidden), (1, 6, false));
    assert_eq!(card.title.as_option().unwrap().text.as_option().unwrap().en, "Bug Hunter");
    assert_eq!(card.showcase.len(), 2);

    let profile: IdentityProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            Some(&bob),
            &IdentityProfileRequest {
                by: Some(get_profile_request::By::UserId(alice.id.to_string())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let cards = &profile.profile.as_option().unwrap().cards;
    let level = cards.iter().find(|card| card.kind == "quest.level").expect("the quest.level card");
    assert_eq!(level.title.as_option().unwrap().ja, "レベル1");
    assert_eq!(level.text.as_option().unwrap().ja, "バグハンター");
    assert_eq!(level.data["level"], "1");
    assert_eq!(level.data["title_badge"], "bug_hunter");
    let showcased: serde_json::Value = serde_json::from_str(&level.data["badges"]).unwrap();
    assert_eq!(
        showcased.as_array().unwrap().iter().map(|badge| badge["key"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["early_adopter", "bug_hunter"]
    );
    assert_eq!(level.path, format!("/u/{}/quest", alice.handle));

    // An empty key removes the title, an empty list the showcase.
    assert!(set_title(&ctx, &alice, "").await.unwrap().title.as_option().is_none());
    assert!(set_showcase(&ctx, &alice, &[]).await.unwrap().showcase.is_empty());
}

#[tokio::test]
async fn a_player_can_keep_their_progress_off_their_profile() {
    let ctx = start().await;
    let (alice, bob, blocked) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("blocked").await);
    badges::award(&ctx.app.core, alice.id, "bug_hunter").await.unwrap();

    let initial: GetPreferencesResponse =
        ctx.call(QUEST, "GetPreferences", Some(&alice), &GetPreferencesRequest::default()).await.unwrap();
    let initial = initial.preferences.as_option().unwrap();
    assert!(initial.show_on_leaderboards && initial.show_on_profile && initial.streak_reminders);

    let updated: UpdatePreferencesResponse = ctx
        .call(
            QUEST,
            "UpdatePreferences",
            Some(&alice),
            &UpdatePreferencesRequest { show_on_profile: Some(false), ..Default::default() },
        )
        .await
        .unwrap();
    let updated = updated.preferences.as_option().unwrap();
    assert!(
        !updated.show_on_profile && updated.show_on_leaderboards && updated.streak_reminders,
        "unset fields keep their value"
    );

    let seen = user_badges(&ctx, Some(&bob), &alice).await.unwrap();
    assert!(seen.hidden && seen.badges.is_empty() && seen.showcase.is_empty());
    let card: GetProfileResponse = ctx
        .call(
            QUEST,
            "GetProfile",
            Some(&bob),
            &GetProfileRequest { user_id: alice.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let card = card.profile.as_option().unwrap();
    assert!(card.hidden && card.level == 0 && card.showcase.is_empty());
    assert_eq!(card.user.as_option().unwrap().id, alice.id.to_string());
    let profile: IdentityProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            Some(&bob),
            &IdentityProfileRequest {
                by: Some(get_profile_request::By::UserId(alice.id.to_string())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!profile.profile.as_option().unwrap().cards.iter().any(|card| card.kind == "quest.level"));
    // What she earns from now on stays out of her friends' feeds.
    badges::award(&ctx.app.core, alice.id, "early_adopter").await.unwrap();
    let activities = ctx.app.events_by("activity.recorded", alice.id).await;
    assert_eq!(activities.last().unwrap().data["audience"], "private");
    // Alice herself still sees everything.
    assert_eq!(user_badges(&ctx, Some(&alice), &alice).await.unwrap().badges.len(), 2);

    // A block hides the player altogether; unknown players do not exist.
    let block = tokuly_proto::social::BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
    let _: tokuly_proto::social::BlockUserResponse =
        ctx.app.call("tokuly.social.v1.FriendService/BlockUser", Some(&alice), &block).await.unwrap();
    assert_error(user_badges(&ctx, Some(&blocked), &alice).await, "not_found", "USER_NOT_FOUND");
    assert_error(
        ctx.call::<_, GetProfileResponse>(
            QUEST,
            "GetProfile",
            Some(&blocked),
            &GetProfileRequest { user_id: alice.id.to_string(), ..Default::default() },
        )
        .await,
        "not_found",
        "USER_NOT_FOUND",
    );
    assert_error(
        ctx.call::<_, GetProfileResponse>(
            QUEST,
            "GetProfile",
            Some(&bob),
            &GetProfileRequest { user_id: Uuid::now_v7().to_string(), ..Default::default() },
        )
        .await,
        "not_found",
        "USER_NOT_FOUND",
    );
    assert_error(
        ctx.call::<_, ListUserBadgesResponse>(BADGES, "ListUserBadges", None, &ListUserBadgesRequest::default()).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
    assert_error(
        ctx.call::<_, GetPreferencesResponse>(QUEST, "GetPreferences", None, &GetPreferencesRequest::default()).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
    assert_error(
        ctx.call::<_, UpdatePreferencesResponse>(
            QUEST,
            "UpdatePreferences",
            None,
            &UpdatePreferencesRequest::default(),
        )
        .await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );
}

#[tokio::test]
async fn rarity_is_the_share_of_active_players_owning_a_badge() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    ctx.send_stamps(&alice, 1).await;
    ctx.flush(&[&alice]).await;
    badges::award(&ctx.app.core, alice.id, "event_winner").await.unwrap();
    // A share is only published once enough players are active for it to
    // say something other than who owns the badge: make sure there are.
    let prefix = format!("crowd{}_", tokuly_core::ids::random_base62(8).to_ascii_lowercase());
    sqlx::query(
        "WITH crowd AS (INSERT INTO users (handle, display_name) SELECT ($1 || n)::citext, 'Crowd' FROM generate_series(1, 12) AS n RETURNING id)
         INSERT INTO quest_profiles (user_id, last_active_day) SELECT id, $2 FROM crowd",
    )
    .bind(prefix)
    .bind(today())
    .execute(&ctx.app.core.db)
    .await
    .unwrap();

    ctx.quest.recompute_rarity().await.unwrap();
    let catalog = catalog(&ctx, Some(&alice)).await;
    let owned = find(&catalog, "event_winner");
    assert!(
        owned.owner_share > 0.0 && owned.owner_share < 1.0,
        "one of at least thirteen active players owns it: {}",
        owned.owner_share
    );
    assert_ne!(owned.rarity, BadgeRarity::BADGE_RARITY_UNSPECIFIED);
    // Nobody has kept a streak for a thousand days.
    let unowned = find(&catalog, "streak_1000");
    assert_eq!((unowned.owner_share, unowned.rarity.as_known()), (0.0, Some(BadgeRarity::BADGE_RARITY_LEGENDARY)));
}

#[tokio::test]
async fn staff_award_and_revoke_badges_by_hand() {
    let ctx = start().await;
    let (alice, admin) = (ctx.app.user("alice").await, ctx.app.user_with_roles("admin", &["admin"]).await);
    let award = AwardBadgeRequest {
        user_id: alice.id.to_string(),
        badge_key: "early_adopter".into(),
        reason: "spring event prize".into(),
        ..Default::default()
    };
    let revoke = RevokeBadgeRequest {
        user_id: alice.id.to_string(),
        badge_key: "early_adopter".into(),
        reason: "spring event prize".into(),
        ..Default::default()
    };

    assert_error(
        ctx.call::<_, AwardBadgeResponse>(ADMIN, "AwardBadge", Some(&alice), &award).await,
        "permission_denied",
        "FORBIDDEN",
    );
    assert_error(
        ctx.call::<_, RevokeBadgeResponse>(ADMIN, "RevokeBadge", Some(&alice), &revoke).await,
        "permission_denied",
        "FORBIDDEN",
    );
    assert_error(
        ctx.call::<_, AwardBadgeResponse>(ADMIN, "AwardBadge", None, &award).await,
        "unauthenticated",
        "SIGN_IN_REQUIRED",
    );

    // Staff do not decorate themselves.
    let own = AwardBadgeRequest {
        user_id: admin.id.to_string(),
        badge_key: "early_adopter".into(),
        reason: "spring event prize".into(),
        ..Default::default()
    };
    assert_error(
        ctx.call::<_, AwardBadgeResponse>(ADMIN, "AwardBadge", Some(&admin), &own).await,
        "permission_denied",
        "SELF_AWARD",
    );
    let own = RevokeBadgeRequest {
        user_id: admin.id.to_string(),
        badge_key: "early_adopter".into(),
        reason: "spring event prize".into(),
        ..Default::default()
    };
    assert_error(
        ctx.call::<_, RevokeBadgeResponse>(ADMIN, "RevokeBadge", Some(&admin), &own).await,
        "permission_denied",
        "SELF_AWARD",
    );
    assert!(ctx.badge_keys(&admin).await.is_empty());

    let awarded: AwardBadgeResponse = ctx.call(ADMIN, "AwardBadge", Some(&admin), &award).await.unwrap();
    assert!(awarded.awarded && awarded.badge.as_option().unwrap().owned);
    let again: AwardBadgeResponse = ctx.call(ADMIN, "AwardBadge", Some(&admin), &award).await.unwrap();
    assert!(!again.awarded);
    set_showcase(&ctx, &alice, &["early_adopter"]).await.unwrap();
    set_title(&ctx, &alice, "early_adopter").await.unwrap();

    let revoked: RevokeBadgeResponse = ctx.call(ADMIN, "RevokeBadge", Some(&admin), &revoke).await.unwrap();
    assert!(revoked.revoked);
    let after = user_badges(&ctx, Some(&alice), &alice).await.unwrap();
    assert!(
        after.badges.is_empty() && after.showcase.is_empty() && after.title.as_option().is_none(),
        "gone from the showcase and the title too"
    );
    let again: RevokeBadgeResponse = ctx.call(ADMIN, "RevokeBadge", Some(&admin), &revoke).await.unwrap();
    assert!(!again.revoked);

    let unknown_badge = AwardBadgeRequest {
        user_id: alice.id.to_string(),
        badge_key: "no_such_badge".into(),
        reason: "typo".into(),
        ..Default::default()
    };
    assert_error(
        ctx.call::<_, AwardBadgeResponse>(ADMIN, "AwardBadge", Some(&admin), &unknown_badge).await,
        "not_found",
        "BADGE_NOT_FOUND",
    );
    let unknown_user = AwardBadgeRequest {
        user_id: Uuid::now_v7().to_string(),
        badge_key: "early_adopter".into(),
        reason: "wrong account".into(),
        ..Default::default()
    };
    assert_error(
        ctx.call::<_, AwardBadgeResponse>(ADMIN, "AwardBadge", Some(&admin), &unknown_user).await,
        "not_found",
        "USER_NOT_FOUND",
    );
    let unknown_user = RevokeBadgeRequest {
        user_id: Uuid::now_v7().to_string(),
        badge_key: "early_adopter".into(),
        reason: "wrong account".into(),
        ..Default::default()
    };
    assert_error(
        ctx.call::<_, RevokeBadgeResponse>(ADMIN, "RevokeBadge", Some(&admin), &unknown_user).await,
        "not_found",
        "USER_NOT_FOUND",
    );

    // No badge moves by hand without a reason.
    for reason in ["", "   "] {
        let silent = AwardBadgeRequest { reason: reason.into(), ..award.clone() };
        assert_error(
            ctx.call::<_, AwardBadgeResponse>(ADMIN, "AwardBadge", Some(&admin), &silent).await,
            "invalid_argument",
            "REASON_REQUIRED",
        );
        let silent = RevokeBadgeRequest { reason: reason.into(), ..revoke.clone() };
        assert_error(
            ctx.call::<_, RevokeBadgeResponse>(ADMIN, "RevokeBadge", Some(&admin), &silent).await,
            "invalid_argument",
            "REASON_REQUIRED",
        );
    }
    let wordy = AwardBadgeRequest { reason: "あ".repeat(501), ..award.clone() };
    assert_error(
        ctx.call::<_, AwardBadgeResponse>(ADMIN, "AwardBadge", Some(&admin), &wordy).await,
        "invalid_argument",
        "REASON_TOO_LONG",
    );

    // Who did what to whom, and why — including the calls that changed nothing.
    let list = ListBadgeActionsRequest { user_id: alice.id.to_string(), ..Default::default() };
    assert_error(
        ctx.call::<_, ListBadgeActionsResponse>(ADMIN, "ListBadgeActions", Some(&alice), &list).await,
        "permission_denied",
        "FORBIDDEN",
    );
    let log: ListBadgeActionsResponse = ctx.call(ADMIN, "ListBadgeActions", Some(&admin), &list).await.unwrap();
    let seen: Vec<(Option<BadgeActionKind>, bool, &str, &str)> = log
        .actions
        .iter()
        .rev()
        .map(|action| (action.kind.as_known(), action.changed, action.reason.as_str(), action.staff_id.as_str()))
        .collect();
    let staff = admin.id.to_string();
    assert_eq!(
        seen,
        vec![
            (Some(BadgeActionKind::BADGE_ACTION_KIND_AWARDED), true, "spring event prize", staff.as_str()),
            (Some(BadgeActionKind::BADGE_ACTION_KIND_AWARDED), false, "spring event prize", staff.as_str()),
            (Some(BadgeActionKind::BADGE_ACTION_KIND_REVOKED), true, "spring event prize", staff.as_str()),
            (Some(BadgeActionKind::BADGE_ACTION_KIND_REVOKED), false, "spring event prize", staff.as_str()),
        ]
    );
    assert!(
        log.actions.iter().all(|action| action.badge_key == "early_adopter" && action.user_id == alice.id.to_string())
    );
    // Refused requests leave no record.
    let all: ListBadgeActionsResponse =
        ctx.call(ADMIN, "ListBadgeActions", Some(&admin), &ListBadgeActionsRequest::default()).await.unwrap();
    assert!(
        !all.actions.iter().any(|action| action.user_id == admin.id.to_string() || action.badge_key == "no_such_badge")
    );
}

#[tokio::test]
async fn a_badge_of_the_hour_on_the_first_day_comes_without_a_notification() {
    let ctx = start().await;
    // Six in the morning where Alice lives, on the day she signed up: whatever she does
    // first earns the early-bird badge, but she did nothing to be told about.
    let alice = ctx.app.user("alice").await;
    ctx.set_zone(&alice, &zone_at_hour(6)).await;
    ctx.send_stamps(&alice, 1).await;
    ctx.flush(&[&alice]).await;
    assert!(ctx.badge_keys(&alice).await.contains(&"early_bird".to_owned()));
    let notices = ctx.notifications(&alice, "quest.badge").await;
    assert!(notices.iter().all(|notice| notice["data"]["badge_key"] != "early_bird"), "{notices:?}");
}

#[tokio::test]
async fn a_birthday_earns_its_secret_badge() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // Today is Alice's birthday, by her own calendar.
    let (month, day) = (chrono::Datelike::month(&today()), chrono::Datelike::day(&today()));
    let born =
        chrono::NaiveDate::from_ymd_opt(2008, month, day).expect("2008 was a leap year: every day of the year exists");
    sqlx::query("UPDATE users SET birthdate = $2 WHERE id = $1")
        .bind(alice.id)
        .bind(born)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.send_stamps(&alice, 1).await;
    ctx.flush(&[&alice]).await;
    assert!(ctx.badge_keys(&alice).await.contains(&"birthday".to_owned()));
    // Somebody whose birthday is another day gets nothing for it.
    let bob = ctx.app.user("bob").await;
    sqlx::query("UPDATE users SET birthdate = $2::date - 40 WHERE id = $1")
        .bind(bob.id)
        .bind(today())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.send_stamps(&bob, 1).await;
    ctx.flush(&[&bob]).await;
    assert!(!ctx.badge_keys(&bob).await.contains(&"birthday".to_owned()));
}

#[tokio::test]
async fn badges_a_player_already_qualifies_for_are_caught_up() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // As if the chatter badges had been added after Alice sent 150 messages.
    sqlx::query("INSERT INTO quest_counters (user_id, metric, value) VALUES ($1, 'messages', 150)")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(!ctx.badge_keys(&alice).await.contains(&"chatter_bronze".to_owned()));
    let catalog = catalog(&ctx, Some(&alice)).await;
    assert!(find(&catalog, "chatter_bronze").owned);
    assert!(!find(&catalog, "chatter_silver").owned);
    assert!(ctx.badge_keys(&alice).await.contains(&"chatter_bronze".to_owned()));
}
