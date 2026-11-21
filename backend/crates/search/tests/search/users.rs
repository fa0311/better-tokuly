//! User documents: built from identity's data on identity's events, and the
//! effect of suspensions and deletions on everything a user owns.
use tokuly_core::outbound::SEARCH_TOPIC;
use tokuly_proto::search::*;
use tokuly_testkit::TestUser;
use uuid::Uuid;

use crate::common::*;

/// Searches the user tab.
async fn find_users(ctx: &Ctx, query: &str) -> SearchResponse {
    ctx.search(None, &SearchRequest { query: query.to_owned(), kind: SearchKind::SEARCH_KIND_USER.into(), ..Default::default() }).await
}

async fn set_profile(ctx: &Ctx, user: &TestUser, display_name: &str, bio: &str) {
    sqlx::query("UPDATE users SET display_name = $2, bio = $3 WHERE id = $1")
        .bind(user.id)
        .bind(display_name)
        .bind(bio)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

/// The random part of a test user's handle (`<name>_<suffix>`).
fn handle_suffix(user: &TestUser) -> &str {
    user.handle.rsplit('_').next().unwrap()
}

#[tokio::test]
async fn a_new_user_is_found_by_name_handle_and_bio() {
    let ctx = start().await;
    let user = ctx.app.user("newcomer").await;
    let (name_word, bio_word) = (unique("name"), unique("bio"));
    let name = format!("{name_word} とくぞう");
    set_profile(&ctx, &user, &name, &format!("{bio_word} が好きです")).await;

    ctx.index_user(&user).await;
    ctx.flush().await;

    for query in [name_word.as_str(), user.handle.as_str(), handle_suffix(&user), bio_word.as_str()] {
        let found = find_users(&ctx, query).await;
        assert_eq!(hit_ids(&found.hits), vec![user.id.to_string()], "query {query:?}");
    }
    let found = find_users(&ctx, &name_word).await;
    let hit = &found.hits[0];
    assert_eq!(kind_of(hit), SearchKind::SEARCH_KIND_USER);
    assert_eq!(hit.title, name);
    assert_eq!(hit.path, format!("/u/{}", user.handle));
    let owner = hit.owner.as_option().expect("a user hit carries the user's summary");
    assert_eq!((owner.id.as_str(), owner.handle.as_str()), (user.id.to_string().as_str(), user.handle.as_str()));
    let extra: serde_json::Value = serde_json::from_str(&hit.extra_json).unwrap();
    assert_eq!(extra, serde_json::json!({ "handle": user.handle, "verified": false }));
    assert!(hit.created_at.as_option().is_some());
}

#[tokio::test]
async fn profile_changes_reach_the_index() {
    let ctx = start().await;
    let user = ctx.app.user("renamer").await;
    let (before, after) = (unique("before"), unique("after"));
    set_profile(&ctx, &user, &before, "").await;
    ctx.index_user(&user).await;
    ctx.flush().await;
    assert_eq!(find_users(&ctx, &before).await.hits.len(), 1);

    set_profile(&ctx, &user, &after, "").await;
    ctx.deliver(event("identity.user.updated", user_ref(user.id))).await;
    ctx.flush().await;

    assert!(find_users(&ctx, &before).await.hits.is_empty());
    assert_eq!(hit_ids(&find_users(&ctx, &after).await.hits), vec![user.id.to_string()]);
}

#[tokio::test]
async fn a_user_who_turns_discoverability_off_disappears_and_can_return() {
    let ctx = start().await;
    let user = ctx.app.user("shy").await;
    ctx.index_user(&user).await;
    ctx.flush().await;
    assert_eq!(find_users(&ctx, &user.handle).await.hits.len(), 1);

    let set_discoverable = async |discoverable: bool| {
        sqlx::query("UPDATE users SET privacy = $2 WHERE id = $1")
            .bind(user.id)
            .bind(serde_json::json!({ "discoverable": discoverable, "shareBattery": true }))
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
        ctx.deliver(event("identity.user.privacy_changed", user_ref(user.id))).await;
        ctx.flush().await;
    };

    set_discoverable(false).await;
    assert!(find_users(&ctx, &user.handle).await.hits.is_empty());
    set_discoverable(true).await;
    assert_eq!(find_users(&ctx, &user.handle).await.hits.len(), 1);
}

#[tokio::test]
async fn a_suspension_hides_the_user_and_everything_they_own_until_it_is_lifted() {
    let ctx = start().await;
    let (creator, bystander) = (ctx.app.user("creator").await, ctx.app.user("bystander").await);
    let word = unique("ban");
    let (theirs, others) = (doc("video", Some(creator.id), &word), doc("video", Some(bystander.id), &word));
    ctx.index_user(&creator).await;
    ctx.index(&[theirs.clone(), others.clone()]).await;
    assert_eq!(ctx.find(None, &word).await.hits.len(), 2);
    assert_eq!(find_users(&ctx, &creator.handle).await.hits.len(), 1);

    ctx.app.identity.set_banned(creator.id, Some("spam")).await.unwrap();
    // Before the consumer has even seen the event, results are filtered.
    assert_eq!(hit_ids(&ctx.find(None, &word).await.hits), vec![others.id.to_string()]);
    assert!(find_users(&ctx, &creator.handle).await.hits.is_empty());

    // Then the documents leave the index itself, so counts are right too.
    ctx.deliver(event("identity.user.banned", user_ref(creator.id))).await;
    ctx.flush().await;
    let during = ctx.find(None, &word).await;
    assert_eq!(hit_ids(&during.hits), vec![others.id.to_string()]);
    assert_eq!((during.total, count_of(&during, SearchKind::SEARCH_KIND_VIDEO)), (1, 1));
    // What they publish while suspended is held back as well.
    let late = doc("video", Some(creator.id), &word);
    ctx.index(std::slice::from_ref(&late)).await;
    assert_eq!(ctx.find(None, &word).await.total, 1);

    ctx.app.identity.set_banned(creator.id, None).await.unwrap();
    ctx.deliver(event("identity.user.unbanned", user_ref(creator.id))).await;
    ctx.flush().await;
    let mut after = hit_ids(&ctx.find(None, &word).await.hits);
    after.sort();
    let mut expected = vec![theirs.id.to_string(), others.id.to_string(), late.id.to_string()];
    expected.sort();
    assert_eq!(after, expected, "everything is back, including what was published meanwhile");
    assert_eq!(find_users(&ctx, &creator.handle).await.hits.len(), 1);
}

#[tokio::test]
async fn a_deleted_account_takes_its_documents_and_history_with_it() {
    let ctx = start().await;
    let (leaver, other) = (ctx.app.user("leaver").await, ctx.app.user("stayer").await);
    let word = unique("gone");
    let (theirs, others) = (doc("note", Some(leaver.id), &word), doc("note", Some(other.id), &word));
    let publication = event_with_id(Uuid::now_v7(), SEARCH_TOPIC, upsert_of(&theirs));
    ctx.deliver(publication.clone()).await;
    ctx.index_user(&leaver).await;
    ctx.index(std::slice::from_ref(&others)).await;
    ctx.find(Some(&leaver), &word).await; // remembered in their history
    assert_eq!(ctx.recent(&leaver).await.searches.len(), 1);
    let _: SetHistoryPausedResponse =
        ctx.call(SEARCH, "SetHistoryPaused", Some(&leaver), &SetHistoryPausedRequest { paused: true, ..Default::default() }).await.unwrap();

    // Identity purges the account and announces it.
    sqlx::query("UPDATE users SET deletion_scheduled_at = now() - interval '1 minute' WHERE id = $1")
        .bind(leaver.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.app.identity.purge_deleted_accounts().await.unwrap() >= 1);
    ctx.deliver(event("identity.user.deleted", user_ref(leaver.id))).await;
    ctx.flush().await;

    assert_eq!(hit_ids(&ctx.find(None, &word).await.hits), vec![others.id.to_string()]);
    assert!(find_users(&ctx, &leaver.handle).await.hits.is_empty());
    let (recent, settings): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM search_recent WHERE user_id = $1), (SELECT count(*) FROM search_user_settings WHERE user_id = $1)",
    )
    .bind(leaver.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!((recent, settings), (0, 0));
    let still_owned: i64 = sqlx::query_scalar("SELECT count(*) FROM search_documents WHERE owner_id = $1")
        .bind(leaver.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(still_owned, 0, "the registry forgets whose documents they were");

    // A redelivery of the old publication changes nothing, and neither does
    // a domain that publishes for the deleted account afterwards.
    ctx.deliver(publication).await;
    ctx.publish(&doc("note", Some(leaver.id), &word)).await;
    ctx.deliver(event("identity.user.deleted", user_ref(leaver.id))).await;
    ctx.flush().await;
    assert_eq!(hit_ids(&ctx.find(None, &word).await.hits), vec![others.id.to_string()]);
}

#[tokio::test]
async fn the_user_reindex_backfills_everyone_who_may_be_found() {
    let ctx = start().await;
    let (visible, hidden, banned) = (ctx.app.user("visible").await, ctx.app.user("hidden").await, ctx.app.user("banned").await);
    let word = unique("backfill");
    for user in [&visible, &hidden, &banned] {
        set_profile(&ctx, user, &format!("{word} {}", handle_suffix(user)), "").await;
    }
    // `hidden` was searchable once, and has hidden since without the event
    // ever reaching us.
    ctx.index_user(&hidden).await;
    sqlx::query("UPDATE users SET privacy = '{\"discoverable\": false}' WHERE id = $1").bind(hidden.id).execute(&ctx.app.core.db).await.unwrap();
    ctx.app.identity.set_banned(banned.id, Some("spam")).await.unwrap();

    // None of them was ever announced by an event.
    let report = ctx.search.reindex_users().await.unwrap();
    ctx.flush().await;
    assert!(report.scanned >= 3 && report.stored >= 1, "{report:?}");
    assert_eq!(hit_ids(&find_users(&ctx, &word).await.hits), vec![visible.id.to_string()]);

    // A second run finds nothing to change for them.
    ctx.search.reindex_users().await.unwrap();
    let unwritten: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM search_documents WHERE kind = 'user' AND doc_id = ANY($1) AND synced_generation < generation",
    )
    .bind(vec![visible.id, hidden.id, banned.id])
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(unwritten, 0);
}

#[tokio::test]
async fn an_event_about_an_unknown_user_is_harmless() {
    let ctx = start().await;
    let nobody = Uuid::now_v7();
    for topic in ["identity.user.created", "identity.user.updated", "identity.user.banned", "identity.user.unbanned", "identity.user.deleted"] {
        ctx.deliver(event(topic, user_ref(nobody))).await;
    }
    ctx.flush().await;
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM search_documents WHERE doc_id = $1 AND doc IS NOT NULL")
        .bind(nobody)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(held, 0);
}
