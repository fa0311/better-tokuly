//! How the emoji domain fits into the rest: what it does when an account
//! is erased, an upload is deleted by force or a report is upheld, and what
//! it contributes to reports, data exports and profiles.
mod common;

use common::*;
use tokuly_core::events::Event;
use tokuly_emoji::catalog;
use tokuly_proto::{common::SubjectType, emoji::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn content_removed(subject_type: SubjectType, subject_id: &str, reason: &str, moderator: &TestUser) -> Event {
    event(
        "admin.content.removed",
        serde_json::json!({ "subject_type": subject_type as i32, "subject_id": subject_id, "reason": reason, "moderator_id": moderator.id }),
    )
}

async fn count(ctx: &Ctx, sql: &'static str, user: &TestUser) -> i64 {
    sqlx::query_scalar(sql).bind(user.id).fetch_one(&ctx.app.core.db).await.unwrap()
}

async fn asset_of(ctx: &Ctx, emoji_id: &str) -> String {
    sqlx::query_scalar("SELECT asset_id::text FROM emojis WHERE id = $1::uuid")
        .bind(emoji_id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_erased_account_leaves_its_published_emoji_behind_uncredited() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let published = ctx.submit(&author, &unique("published")).await;
    ctx.approve(&moderator, &published.emoji.id).await;
    let disabled = ctx.submit(&author, &unique("disabled")).await;
    ctx.approve(&moderator, &disabled.emoji.id).await;
    ctx.disable(&moderator, &disabled.emoji.id, "おやすみ").await;
    let waiting = ctx.submit(&author, &unique("waiting")).await;
    let rejected = ctx.submit(&author, &unique("rejected")).await;
    ctx.reject(&moderator, &rejected.emoji.id, "だめ").await;
    let (kept_upload, erased_upload) =
        (asset_of(&ctx, &published.emoji.id).await, asset_of(&ctx, &waiting.emoji.id).await);

    let pinned = SetDeckRequest { emojis: vec![format!(":{}:", published.emoji.name)], ..Default::default() };
    let _: SetDeckResponse = ctx.call("", "SetDeck", Some(&author), &pinned).await.unwrap();
    ctx.record_use(&author, &[&published.emoji.name, "👍"]).await;
    ctx.trust(&moderator, &author, true).await;
    // The author also acted as a fan of somebody else's emoji; that emoji is untouched.
    let foreign = ctx.add(&moderator, &unique("foreign")).await;

    let erased = event("identity.user.deleted", serde_json::json!({ "user_id": author.id }));
    ctx.emoji.handle_event(erased.clone()).await.unwrap();

    // What the community uses stays, without the credit.
    let kept = ctx.get(None, &published.emoji.id).await.unwrap();
    assert_eq!(kept.state, EmojiState::EMOJI_STATE_APPROVED);
    assert!(!kept.author.is_set());
    assert!(catalog::exists(&ctx.app.core, &published.emoji.name).await.unwrap());
    assert_eq!(ctx.holders(&kept_upload).await, 1, "and keeps its picture");
    let doc = ctx.search_changes(&published.emoji.id).await.last().unwrap().clone().expect("still searchable");
    assert!(doc["owner_id"].is_null(), "search would hide a deleted owner's documents");
    let still_disabled = ctx.get(Some(&moderator), &disabled.emoji.id).await.unwrap();
    assert_eq!(still_disabled.state, EmojiState::EMOJI_STATE_DISABLED);
    assert!(!still_disabled.author.is_set());
    assert!(still_disabled.history.iter().all(|record| record.actor.id != author.id.to_string()));

    // What was theirs alone is gone.
    for gone in [&waiting, &rejected] {
        assert_eq!(ctx.get(Some(&moderator), &gone.emoji.id).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    }
    assert_eq!(ctx.holders(&erased_upload).await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM emojis WHERE author_id = $1", &author).await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM emoji_decks WHERE user_id = $1", &author).await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM emoji_recents WHERE user_id = $1", &author).await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM emoji_submitters WHERE user_id = $1", &author).await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM emoji_reviews WHERE actor_id = $1", &author).await, 0);
    assert_eq!(ctx.get(None, &foreign.emoji.id).await.unwrap().state, EmojiState::EMOJI_STATE_APPROVED);

    // A moderator's account goes the same way: their decisions stay, unsigned.
    let leaving = ctx.moderator().await;
    let decided = ctx.submit(&ctx.app.user("someone").await, &unique("decided")).await;
    ctx.approve(&leaving, &decided.emoji.id).await;
    ctx.trust(&leaving, &ctx.app.user("trusted").await, true).await;
    ctx.emoji.handle_event(event("identity.user.deleted", serde_json::json!({ "user_id": leaving.id }))).await.unwrap();
    let history = ctx.get(Some(&moderator), &decided.emoji.id).await.unwrap().history;
    assert_eq!(history[0].action, ReviewAction::REVIEW_ACTION_APPROVED);
    assert!(!history[0].actor.is_set());
    assert_eq!(count(&ctx, "SELECT count(*) FROM emoji_submitters WHERE trusted_by = $1", &leaving).await, 0);

    // The event may arrive twice.
    ctx.emoji.handle_event(erased).await.unwrap();
    assert!(catalog::exists(&ctx.app.core, &published.emoji.name).await.unwrap());
}

#[tokio::test]
async fn an_upload_deleted_by_force_takes_its_emoji_along() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let name = unique("broken");
    let emoji = ctx.add(&moderator, &name).await;
    let waiting = ctx.submit(&author, &unique("waiting")).await;
    let (upload, waiting_upload) = (asset_of(&ctx, &emoji.emoji.id).await, asset_of(&ctx, &waiting.emoji.id).await);
    let version = ctx.catalog().await.version;

    // The uploader cannot delete a picture an emoji holds; a takedown can.
    let core = &ctx.app.core;
    let refused = tokuly_media::assets::delete(core, upload.parse().unwrap(), Some(moderator.id)).await.unwrap_err();
    assert_eq!(refused.reason, "ASSET_IN_USE");
    for forced in [&upload, &waiting_upload] {
        tokuly_media::assets::delete(core, forced.parse().unwrap(), None).await.unwrap();
    }
    let deleted: Vec<Event> = ctx
        .app
        .events("media.asset.deleted")
        .await
        .into_iter()
        .filter(|event| [&upload, &waiting_upload].iter().any(|id| event.data["asset_id"] == serde_json::json!(id)))
        .collect();
    assert_eq!(deleted.len(), 2);
    for event in &deleted {
        ctx.emoji.handle_event(event.clone()).await.unwrap();
    }

    // No emoji is left showing a picture that is gone.
    assert_eq!(ctx.get(Some(&moderator), &emoji.emoji.id).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    assert_eq!(ctx.get(Some(&author), &waiting.emoji.id).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    assert!(!catalog::exists(core, &name).await.unwrap());
    assert!(ctx.changes(version).await.removed_ids.contains(&emoji.emoji.id));
    let removed = ctx.emoji_events("emoji.emoji.removed", &emoji.emoji.id).await;
    assert_eq!((removed.len(), removed[0]["because"].as_str()), (1, Some("asset_deleted")));
    assert!(ctx.search_changes(&emoji.emoji.id).await.last().unwrap().is_none());

    // Again, and for uploads no emoji uses: nothing happens.
    ctx.emoji.handle_event(deleted[0].clone()).await.unwrap();
    let unrelated = event(
        "media.asset.deleted",
        serde_json::json!({ "asset_id": Uuid::now_v7(), "owner_id": author.id, "purpose": 4, "kind": 1 }),
    );
    ctx.emoji.handle_event(unrelated).await.unwrap();
    // Uploads becoming ready elsewhere in the app are none of our business either.
    let ready = event(
        "media.asset.ready",
        serde_json::json!({ "asset_id": Uuid::now_v7(), "owner_id": author.id, "purpose": 11, "kind": 1 }),
    );
    ctx.emoji.handle_event(ready).await.unwrap();
    ctx.emoji.handle_event(event("some.other.topic", serde_json::json!({}))).await.unwrap();
}

#[tokio::test]
async fn emoji_can_be_reported_and_an_upheld_report_disables_them() {
    let ctx = start().await;
    let (moderator, author, stranger) =
        (ctx.moderator().await, ctx.app.user("author").await, ctx.app.user("stranger").await);
    let subjects = &ctx.app.core.subjects;
    let kind = SubjectType::SUBJECT_TYPE_CUSTOM_EMOJI;
    let name = unique("reported");
    let emoji = ctx.submit(&author, &name).await;
    let id = emoji.emoji.id.clone();

    // Waiting for review, only its author can refer to it.
    assert_eq!(subjects.resolve(kind, &id, Some(stranger.id)).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    assert_eq!(subjects.resolve(kind, &id, None).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    assert!(subjects.resolve(kind, &id, Some(author.id)).await.is_ok());
    ctx.approve(&moderator, &id).await;

    for reference in [id.as_str(), name.as_str()] {
        let subject = subjects.resolve(kind, reference, Some(stranger.id)).await.unwrap();
        assert_eq!(subject.id.to_string(), id);
        assert_eq!(subject.owner_id, author.id);
        assert_eq!(
            (subject.title.as_str(), subject.path.as_str()),
            (format!(":{name}:").as_str(), format!("/emoji/{name}").as_str())
        );
        assert_eq!(subject.image_url.as_deref(), Some(emoji.emoji.image.url.as_str()));
        assert!(!subject.interactions_enabled, "an emoji is reported, not liked or commented on");
    }
    let official = ctx.add(&moderator, &unique("official")).await;
    assert!(subjects.resolve(kind, &official.emoji.id, None).await.unwrap().owner_id.is_nil(), "nobody to notify");
    assert_eq!(
        subjects.resolve(kind, &Uuid::now_v7().to_string(), None).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
    assert_eq!(subjects.resolve(kind, "not a name!", None).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");

    // Takedowns of other things are not ours, even with a matching ID.
    ctx.emoji
        .handle_event(content_removed(SubjectType::SUBJECT_TYPE_NOTE, &id, "ちがうもの", &moderator))
        .await
        .unwrap();
    ctx.emoji.handle_event(content_removed(kind, &Uuid::now_v7().to_string(), "だれ?", &moderator)).await.unwrap();
    assert!(catalog::exists(&ctx.app.core, &name).await.unwrap());

    let before = ctx.notification_kinds(&author).await;
    let upheld = content_removed(kind, &id, "権利侵害の報告があったよ", &moderator);
    ctx.emoji.handle_event(upheld.clone()).await.unwrap();
    let taken_down = ctx.get(Some(&moderator), &id).await.unwrap();
    assert_eq!(
        (taken_down.state, taken_down.review_note.as_str()),
        (EmojiState::EMOJI_STATE_DISABLED.into(), "権利侵害の報告があったよ")
    );
    assert_eq!(taken_down.history[0].action, ReviewAction::REVIEW_ACTION_TAKEN_DOWN);
    assert_eq!(taken_down.history[0].actor.id, moderator.id.to_string());
    assert!(!catalog::exists(&ctx.app.core, &name).await.unwrap());
    assert!(named(&ctx.catalog().await.emojis, &name).is_none());
    assert_eq!(subjects.resolve(kind, &id, Some(stranger.id)).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    let removed = ctx.emoji_events("emoji.emoji.removed", &id).await;
    assert_eq!((removed.len(), removed[0]["because"].as_str()), (1, Some("taken_down")));
    assert!(ctx.search_changes(&id).await.last().unwrap().is_none());
    assert_eq!(
        ctx.notification_kinds(&author).await,
        before,
        "the admin domain tells the owner; we do not tell them twice"
    );
    // The name stays with the disabled emoji.
    let squat = ctx.submit_request(&stranger, &name).await;
    assert_eq!(ctx.reason("Submission", "SubmitEmoji", Some(&stranger), &squat).await, "NAME_TAKEN");

    // The event may arrive twice; and a moderator can put the emoji back.
    ctx.emoji.handle_event(upheld).await.unwrap();
    assert_eq!(ctx.get(Some(&moderator), &id).await.unwrap().history.len(), taken_down.history.len());
    assert_eq!(ctx.enable(&moderator, &id).await.state, EmojiState::EMOJI_STATE_APPROVED);

    // A takedown without a stated reason still leaves a note.
    ctx.emoji.handle_event(content_removed(kind, &official.emoji.id, "  ", &moderator)).await.unwrap();
    let noted = ctx.get(Some(&moderator), &official.emoji.id).await.unwrap();
    assert_eq!(noted.state, EmojiState::EMOJI_STATE_DISABLED);
    assert!(!noted.review_note.is_empty());
}

#[tokio::test]
async fn emoji_show_up_in_data_exports_and_on_their_authors_profile() {
    let ctx = start().await;
    let (moderator, author, visitor) =
        (ctx.moderator().await, ctx.app.user("author").await, ctx.app.user("visitor").await);
    let get_profile = async |user: &TestUser| -> serde_json::Value {
        let request = serde_json::json!({ "userId": user.id });
        ctx.app.call("tokuly.identity.v1.ProfileService/GetProfile", Some(&visitor), &request).await.unwrap()
    };
    let card_of = |profile: &serde_json::Value| -> Option<serde_json::Value> {
        profile["profile"]["cards"].as_array()?.iter().find(|card| card["kind"] == "emoji.author").cloned()
    };
    assert!(card_of(&get_profile(&author).await).is_none(), "no emoji, no card");

    let older = ctx.submit(&author, &unique("older")).await;
    ctx.approve(&moderator, &older.emoji.id).await;
    let newer = ctx.submit(&author, &unique("newer")).await;
    ctx.approve(&moderator, &newer.emoji.id).await;
    let waiting = ctx.submit(&author, &unique("waiting")).await;
    let pinned =
        SetDeckRequest { emojis: vec!["🎉".to_owned(), format!(":{}:", older.emoji.name)], ..Default::default() };
    let _: SetDeckResponse = ctx.call("", "SetDeck", Some(&author), &pinned).await.unwrap();
    ctx.record_use(&author, &["👍", &newer.emoji.name]).await;

    // The profile: how many emoji they made, the newest first.
    let card = card_of(&get_profile(&author).await).expect("authors get a card");
    assert_eq!(
        (card["title"]["ja"].as_str(), card["title"]["en"].as_str()),
        (Some("作った絵文字"), Some("Emoji made"))
    );
    assert_eq!((card["text"]["ja"].as_str(), card["text"]["en"].as_str()), (Some("2個"), Some("2 emoji")));
    assert_eq!(card["data"]["count"], "2");
    assert_eq!(
        card["data"]["newest"],
        format!(":{}: :{}:", newer.emoji.name, older.emoji.name),
        "what waits for review is not shown"
    );
    assert_eq!(card["path"], format!("/emoji?author={}", author.id));
    assert_eq!(card["image"]["url"].as_str(), Some(newer.emoji.image.url.as_str()));
    assert!(card_of(&get_profile(&visitor).await).is_none());

    // The data export: everything the domain holds about the user.
    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "emoji")
        .expect("the exporter is registered");
    let data = exporter.export(author.id).await.unwrap();
    let names: Vec<&str> =
        data["emojis"].as_array().unwrap().iter().map(|emoji| emoji["name"].as_str().unwrap()).collect();
    assert_eq!(names, [older.emoji.name.as_str(), newer.emoji.name.as_str(), waiting.emoji.name.as_str()]);
    assert_eq!(data["emojis"][2]["state"], 1);
    assert_eq!(data["deck"], serde_json::json!(["🎉", format!(":{}:", older.emoji.name)]));
    let used: Vec<&str> =
        data["recently_used"].as_array().unwrap().iter().map(|item| item["emoji"].as_str().unwrap()).collect();
    assert_eq!(used, [format!(":{}:", newer.emoji.name).as_str(), "👍"]);
    assert_eq!(data["trusted_submitter"], false);
    let nothing = exporter.export(visitor.id).await.unwrap();
    assert!(nothing["emojis"].as_array().unwrap().is_empty() && nothing["deck"].is_null());
}
