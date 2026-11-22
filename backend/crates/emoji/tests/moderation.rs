//! What moderators do to emoji directly: adding, editing, disabling,
//! enabling, deleting, bulk changes and the listing of every emoji — and
//! that nobody else can.
mod common;

use common::*;
use tokuly_emoji::{EmojiConfig, catalog};
use tokuly_proto::emoji::{__buffa::oneof::bulk_update_emoji_request::Change, __buffa::oneof::emoji_event, *};
use tokuly_testkit::TestUser;

fn aliases(items: &[&str]) -> Box<AliasList> {
    Box::new(AliasList { aliases: strings(items), ..Default::default() })
}

async fn bulk(ctx: &Ctx, moderator: &TestUser, emoji_ids: &[&str], change: Change) -> Result<i32, String> {
    let request = BulkUpdateEmojiRequest { emoji_ids: strings(emoji_ids), change: Some(change), ..Default::default() };
    match ctx.call::<_, BulkUpdateEmojiResponse>("Admin", "BulkUpdateEmoji", Some(moderator), &request).await {
        Ok(response) => Ok(response.updated_count),
        Err(error) => Err(error.reason),
    }
}

async fn list(ctx: &Ctx, moderator: &TestUser, request: ListEmojiRequest) -> ListEmojiResponse {
    ctx.call("Admin", "ListEmoji", Some(moderator), &request).await.unwrap()
}

fn actions(detail: &EmojiDetail) -> Vec<ReviewAction> {
    detail.history.iter().map(|record| record.action.as_known().unwrap()).collect()
}

#[tokio::test]
async fn every_moderation_rpc_needs_the_moderator_role() {
    let ctx = start().await;
    let user = ctx.app.user("user").await;
    let methods = [
        "ListReviewQueue",
        "ApproveEmoji",
        "RejectEmoji",
        "ListEmoji",
        "AddEmoji",
        "UpdateEmoji",
        "DisableEmoji",
        "EnableEmoji",
        "DeleteEmoji",
        "BulkUpdateEmoji",
        "BulkDeleteEmoji",
        "SetSubmitterTrusted",
        "ListTrustedSubmitters",
        "CreateCategory",
        "RenameCategory",
        "ReorderCategories",
        "DeleteCategory",
        "StartImport",
        "GetImport",
        "ListImports",
        "StartExport",
        "GetExport",
        "ListExports",
    ];
    let empty = serde_json::json!({});
    for method in methods {
        assert_eq!(ctx.reason("Admin", method, Some(&user), &empty).await, "FORBIDDEN", "{method} as a user");
        assert_eq!(ctx.reason("Admin", method, None, &empty).await, "SIGN_IN_REQUIRED", "{method} anonymously");
    }
    // Admins hold every role.
    let admin = ctx.app.user_with_roles("admin", &["admin"]).await;
    let queue: ListReviewQueueResponse = ctx.call("Admin", "ListReviewQueue", Some(&admin), &empty).await.unwrap();
    assert!(queue.pending_count >= 0);
    let added = ctx.add(&admin, &unique("by_admin")).await;
    assert_eq!(added.state, EmojiState::EMOJI_STATE_APPROVED);
}

#[tokio::test]
async fn moderators_add_emoji_straight_to_the_catalogue() {
    let reserved = unique("mascot");
    let ctx = start_with(EmojiConfig { reserved_names: vec![reserved.clone()], ..Default::default() }).await;
    let (moderator, artist) = (ctx.moderator().await, ctx.app.user("artist").await);

    // Reserved names are for exactly this; the artist is credited.
    let request = AddEmojiRequest {
        aliases: strings(&["Mascot"]),
        license: "公式".to_owned(),
        local_only: true,
        author_id: artist.id.to_string(),
        ..ctx.add_request(&moderator, &reserved).await
    };
    let picture = request.asset_id.clone();
    let added = ctx.add_with(&moderator, request.clone()).await;
    assert_eq!(added.state, EmojiState::EMOJI_STATE_APPROVED);
    assert_eq!(added.author.id, artist.id.to_string());
    assert!(added.emoji.local_only && added.emoji.added_at.is_set());
    assert_eq!(actions(&added), [ReviewAction::REVIEW_ACTION_ADDED]);
    assert_eq!(added.history[0].actor.id, moderator.id.to_string());
    assert_eq!(ctx.holders(&picture).await, 1);
    assert!(catalog::exists(&ctx.app.core, &reserved).await.unwrap());
    let events = ctx.emoji_events("emoji.emoji.added", &added.emoji.id).await;
    assert_eq!((events.len(), events[0]["via"].as_str()), (1, Some("staff")));
    assert_eq!(
        ctx.app.events_by("activity.recorded", artist.id).await.len(),
        1,
        "the credited artist's friends hear of it"
    );
    assert!(ctx.search_changes(&added.emoji.id).await.last().unwrap().is_some());

    // Without a credit the emoji belongs to nobody.
    let plain = ctx.add(&moderator, &unique("plain")).await;
    assert!(!plain.author.is_set());

    let refuse = async |request: AddEmojiRequest| ctx.reason("Admin", "AddEmoji", Some(&moderator), &request).await;
    assert_eq!(refuse(request.clone()).await, "NAME_TAKEN");
    let fresh = AddEmojiRequest { name: unique("fresh"), ..request.clone() };
    assert_eq!(refuse(AddEmojiRequest { name: "no good".to_owned(), ..fresh.clone() }).await, "NAME_INVALID");
    assert_eq!(
        refuse(AddEmojiRequest { author_id: uuid::Uuid::now_v7().to_string(), ..fresh.clone() }).await,
        "USER_NOT_FOUND"
    );
    assert_eq!(refuse(AddEmojiRequest { author_id: "nope".to_owned(), ..fresh.clone() }).await, "INVALID_ID");
    assert_eq!(
        refuse(AddEmojiRequest { asset_id: ctx.picture(&artist).await, ..fresh.clone() }).await,
        "ASSET_NOT_FOUND",
        "the picture must be the moderator's own upload"
    );
    assert_eq!(refuse(AddEmojiRequest { category: unique("nocategory"), ..fresh.clone() }).await, "CATEGORY_NOT_FOUND");
    assert_eq!(refuse(AddEmojiRequest { aliases: strings(&[""]), ..fresh.clone() }).await, "ALIASES_INVALID");
    assert_eq!(refuse(AddEmojiRequest { license: "x".repeat(1025), ..fresh }).await, "LICENSE_TOO_LONG");
}

#[tokio::test]
async fn a_moderators_edit_reaches_the_catalogue_decks_and_recently_used_lists() {
    let ctx = start().await;
    let (moderator, author, fan) = (ctx.moderator().await, ctx.app.user("author").await, ctx.app.user("fan").await);
    let old_name = unique("typo");
    let emoji = ctx.submit(&author, &old_name).await;
    let id = emoji.emoji.id.clone();
    ctx.approve(&moderator, &id).await;

    // A fan pins and uses it.
    let pinned = SetDeckRequest { emojis: vec![format!(":{old_name}:"), "👍".to_owned()], ..Default::default() };
    let _: SetDeckResponse = ctx.call("", "SetDeck", Some(&fan), &pinned).await.unwrap();
    ctx.record_use(&fan, &[&format!(":{old_name}:")]).await;

    let mut inbox = ctx.app.listen_user(author.id).await;
    let new_name = unique("fixed");
    let new_picture = ctx.picture(&moderator).await;
    let old_picture: String = sqlx::query_scalar("SELECT asset_id::text FROM emojis WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    let change = UpdateEmojiRequest {
        emoji_id: old_name.clone(),
        name: Some(new_name.clone()),
        asset_id: Some(new_picture.clone()),
        aliases: Some(AliasList { aliases: strings(&["fixed", "なおした"]), ..Default::default() }).into(),
        license: Some("CC0".to_owned()),
        sensitive: Some(true),
        local_only: Some(true),
        ..Default::default()
    };
    let updated = ctx.update(&moderator, change).await;
    assert_eq!(updated.emoji.id, id);
    assert_eq!((updated.emoji.name.as_str(), updated.license.as_str()), (new_name.as_str(), "CC0"));
    assert!(updated.emoji.sensitive && updated.emoji.local_only);
    assert_eq!(updated.emoji.aliases, strings(&["fixed", "なおした"]));
    assert_eq!(actions(&updated)[0], ReviewAction::REVIEW_ACTION_EDITED);
    assert_eq!(updated.history[0].note, "name, picture, aliases, license, sensitive, local_only");
    assert_eq!((ctx.holders(&old_picture).await, ctx.holders(&new_picture).await), (0, 1));

    // Everyone's copy follows.
    let catalogue = ctx.catalog().await;
    assert!(named(&catalogue.emojis, &old_name).is_none());
    assert_eq!(named(&catalogue.emojis, &new_name).unwrap().id, id);
    assert!(!catalog::exists(&ctx.app.core, &old_name).await.unwrap());
    let deck: GetDeckResponse = ctx.call("", "GetDeck", Some(&fan), &GetDeckRequest::default()).await.unwrap();
    assert_eq!(
        deck.items.iter().map(|item| item.emoji.as_str()).collect::<Vec<_>>(),
        [format!(":{new_name}:").as_str(), "👍"]
    );
    assert_eq!(deck.items[0].custom.id, id);
    let recent: ListRecentEmojiResponse =
        ctx.call("", "ListRecentEmoji", Some(&fan), &ListRecentEmojiRequest::default()).await.unwrap();
    assert_eq!(recent.items[0].emoji, format!(":{new_name}:"));
    assert_eq!(recent.items[0].custom.id, id);

    let changes = ctx.emoji_events("emoji.emoji.updated", &id).await;
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0]["name"].as_str(), changes[0]["previous_name"].as_str()),
        (Some(new_name.as_str()), Some(old_name.as_str()))
    );
    let doc = ctx.search_changes(&id).await.last().unwrap().clone().unwrap();
    assert_eq!(doc["title"], new_name);
    let pushed = next_matching(&mut inbox, |frame| match emoji_event(frame)? {
        emoji_event::Event::SubmissionUpdated(detail) if detail.emoji.id == id => Some(detail.emoji.name.clone()),
        _ => None,
    })
    .await;
    assert_eq!(pushed.as_deref(), Some(new_name.as_str()), "the author sees what became of their emoji");

    // The credit can be moved or removed; a change of nothing is no change.
    let uncredited = ctx
        .update(
            &moderator,
            UpdateEmojiRequest { emoji_id: id.clone(), author_id: Some(String::new()), ..Default::default() },
        )
        .await;
    assert!(!uncredited.author.is_set());
    assert_eq!(uncredited.history[0].note, "author");
    let records = uncredited.history.len();
    let same = ctx
        .update(
            &moderator,
            UpdateEmojiRequest { emoji_id: id.clone(), license: Some("CC0".to_owned()), ..Default::default() },
        )
        .await;
    assert_eq!(same.history.len(), records, "nothing changed, nothing recorded");
    let credited = ctx
        .update(
            &moderator,
            UpdateEmojiRequest { emoji_id: id.clone(), author_id: Some(fan.id.to_string()), ..Default::default() },
        )
        .await;
    assert_eq!(credited.author.id, fan.id.to_string());

    let other = ctx.add(&moderator, &unique("other")).await;
    let refuse =
        async |request: UpdateEmojiRequest| ctx.reason("Admin", "UpdateEmoji", Some(&moderator), &request).await;
    let on_it = UpdateEmojiRequest { emoji_id: id.clone(), ..Default::default() };
    assert_eq!(
        refuse(UpdateEmojiRequest { name: Some(other.emoji.name.clone()), ..on_it.clone() }).await,
        "NAME_TAKEN"
    );
    assert_eq!(refuse(UpdateEmojiRequest { name: Some("!".to_owned()), ..on_it.clone() }).await, "NAME_INVALID");
    assert_eq!(
        refuse(UpdateEmojiRequest { author_id: Some(uuid::Uuid::now_v7().to_string()), ..on_it.clone() }).await,
        "USER_NOT_FOUND"
    );
    assert_eq!(
        refuse(UpdateEmojiRequest { category: Some(unique("nocategory")), ..on_it.clone() }).await,
        "CATEGORY_NOT_FOUND"
    );
    assert_eq!(
        refuse(UpdateEmojiRequest { asset_id: Some(ctx.picture(&fan).await), ..on_it.clone() }).await,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(refuse(UpdateEmojiRequest { emoji_id: unique("nobody"), ..on_it }).await, "EMOJI_NOT_FOUND");
}

#[tokio::test]
async fn a_disabled_emoji_keeps_its_name_and_can_come_back() {
    let ctx = start().await;
    let (moderator, author, stranger) =
        (ctx.moderator().await, ctx.app.user("author").await, ctx.app.user("stranger").await);
    let name = unique("paused");
    let emoji = ctx.submit(&author, &name).await;
    let id = emoji.emoji.id.clone();

    let disable =
        DisableEmojiRequest {
            emoji_id: id.clone(), reason: "作者に確認中だよ".to_owned(), ..Default::default()
        };
    assert_eq!(
        ctx.reason("Admin", "DisableEmoji", Some(&moderator), &disable).await,
        "EMOJI_NOT_APPROVED",
        "it is still waiting"
    );
    ctx.approve(&moderator, &id).await;
    let no_reason = DisableEmojiRequest { reason: " ".to_owned(), ..disable.clone() };
    assert_eq!(ctx.reason("Admin", "DisableEmoji", Some(&moderator), &no_reason).await, "REASON_INVALID");
    let enable = EnableEmojiRequest { emoji_id: id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "EnableEmoji", Some(&moderator), &enable).await, "EMOJI_NOT_DISABLED");

    let disabled = ctx.disable(&moderator, &id, "作者に確認中だよ").await;
    assert_eq!(
        (disabled.state, disabled.review_note.as_str()),
        (EmojiState::EMOJI_STATE_DISABLED.into(), "作者に確認中だよ")
    );
    assert_eq!(ctx.reason("Admin", "DisableEmoji", Some(&moderator), &disable).await, "EMOJI_NOT_APPROVED");

    // Gone for everyone but the author and the moderators.
    assert!(named(&ctx.catalog().await.emojis, &name).is_none());
    assert!(!catalog::exists(&ctx.app.core, &name).await.unwrap());
    assert_eq!(ctx.get(Some(&stranger), &name).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    let mine = ctx.get(Some(&author), &name).await.unwrap();
    assert_eq!((mine.state, mine.review_note.as_str()), (EmojiState::EMOJI_STATE_DISABLED.into(), "作者に確認中だよ"));
    assert!(!mine.can_edit, "what a moderator disabled is not the author's to change");
    // The name stays taken, so nothing else can pass for it.
    let squat = ctx.submit_request(&stranger, &name).await;
    assert_eq!(ctx.reason("Submission", "SubmitEmoji", Some(&stranger), &squat).await, "NAME_TAKEN");

    let notices = ctx.notifications(&author).await;
    assert_eq!(
        notices.iter().map(|n| n["kind"].as_str().unwrap()).collect::<Vec<_>>(),
        ["emoji.approved", "emoji.disabled"]
    );
    assert!(notices[1]["body"]["ja"].as_str().unwrap().contains("作者に確認中だよ"));
    let removed = ctx.emoji_events("emoji.emoji.removed", &id).await;
    assert_eq!((removed.len(), removed[0]["because"].as_str()), (1, Some("disabled")));
    assert!(ctx.search_changes(&id).await.last().unwrap().is_none(), "it left the search index");

    let enabled = ctx.enable(&moderator, &id).await;
    assert_eq!((enabled.state, enabled.review_note.as_str()), (EmojiState::EMOJI_STATE_APPROVED.into(), ""));
    assert_eq!(
        actions(&enabled),
        [
            ReviewAction::REVIEW_ACTION_ENABLED,
            ReviewAction::REVIEW_ACTION_DISABLED,
            ReviewAction::REVIEW_ACTION_APPROVED,
            ReviewAction::REVIEW_ACTION_SUBMITTED
        ]
    );
    assert!(named(&ctx.catalog().await.emojis, &name).is_some());
    assert!(catalog::exists(&ctx.app.core, &name).await.unwrap());
    let added = ctx.emoji_events("emoji.emoji.added", &id).await;
    assert_eq!(added.iter().map(|event| event["via"].as_str().unwrap()).collect::<Vec<_>>(), ["review", "enabled"]);
    assert!(ctx.search_changes(&id).await.last().unwrap().is_some());
    assert_eq!(
        ctx.app.events_by("activity.recorded", author.id).await.len(),
        1,
        "coming back is not news for the feed"
    );
    assert_eq!(ctx.notification_kinds(&author).await.len(), 2);
}

#[tokio::test]
async fn deleting_an_emoji_frees_its_name_and_its_upload() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let name = unique("doomed");
    let request = ctx.add_request(&moderator, &name).await;
    let picture = request.asset_id.clone();
    let emoji = ctx.add_with(&moderator, request).await;
    let version = ctx.catalog().await.version;

    let delete = DeleteEmojiRequest { emoji_id: emoji.emoji.id.clone(), ..Default::default() };
    let _: DeleteEmojiResponse = ctx.call("Admin", "DeleteEmoji", Some(&moderator), &delete).await.unwrap();
    assert_eq!(ctx.reason("Admin", "DeleteEmoji", Some(&moderator), &delete).await, "EMOJI_NOT_FOUND");
    assert_eq!(ctx.get(Some(&moderator), &emoji.emoji.id).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    assert!(!catalog::exists(&ctx.app.core, &name).await.unwrap());
    assert!(ctx.changes(version).await.removed_ids.contains(&emoji.emoji.id), "clients learn that it is gone");
    assert_eq!(ctx.holders(&picture).await, 0);
    let removed = ctx.emoji_events("emoji.emoji.removed", &emoji.emoji.id).await;
    assert_eq!((removed.len(), removed[0]["because"].as_str()), (1, Some("deleted")));
    assert!(ctx.search_changes(&emoji.emoji.id).await.last().unwrap().is_none());
    // The name can be used again.
    assert_eq!(ctx.add(&moderator, &name).await.emoji.name, name);

    // An emoji that never was in the catalogue leaves no trace there.
    let waiting = ctx.submit(&author, &unique("waiting")).await;
    ctx.delete(&moderator, &waiting.emoji.name).await;
    assert_eq!(ctx.get(Some(&author), &waiting.emoji.id).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    assert!(ctx.emoji_events("emoji.emoji.removed", &waiting.emoji.id).await.is_empty());
    assert!(!ctx.changes(version).await.removed_ids.contains(&waiting.emoji.id));
}

#[tokio::test]
async fn one_change_is_applied_to_many_emoji_at_once() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let category = unique("まとめて");
    ctx.create_category(&moderator, &category).await;
    let first = ctx.add(&moderator, &unique("bulk")).await.emoji.id.clone();
    let second = ctx.add(&moderator, &unique("bulk")).await.emoji.id.clone();
    let waiting = ctx.submit(&author, &unique("bulk")).await.emoji.id.clone();
    let missing = uuid::Uuid::now_v7().to_string();
    let all = [first.as_str(), second.as_str(), waiting.as_str(), missing.as_str()];
    let version = ctx.catalog().await.version;

    assert_eq!(
        bulk(&ctx, &moderator, &all, Change::SetCategory(category.clone())).await,
        Ok(3),
        "unknown IDs are skipped"
    );
    assert_eq!(
        bulk(&ctx, &moderator, &all, Change::SetCategory(category.clone())).await,
        Ok(0),
        "nothing left to change"
    );
    assert_eq!(bulk(&ctx, &moderator, &all, Change::AddAliases(aliases(&["Bulk", "まとめ"]))).await, Ok(3));
    assert_eq!(bulk(&ctx, &moderator, &[&first], Change::RemoveAliases(aliases(&["bulk", "unknown"]))).await, Ok(1));
    assert_eq!(bulk(&ctx, &moderator, &[&second], Change::SetAliases(aliases(&["only"]))).await, Ok(1));
    assert_eq!(bulk(&ctx, &moderator, &all, Change::SetLicense("CC0".to_owned())).await, Ok(3));
    assert_eq!(bulk(&ctx, &moderator, &[&first, &second], Change::SetSensitive(true)).await, Ok(2));
    assert_eq!(bulk(&ctx, &moderator, &[&first], Change::SetLocalOnly(true)).await, Ok(1));

    let one = ctx.get(Some(&moderator), &first).await.unwrap();
    assert_eq!(
        (one.emoji.category.as_str(), one.emoji.aliases.clone(), one.license.as_str()),
        (category.as_str(), strings(&["まとめ"]), "CC0")
    );
    assert!(one.emoji.sensitive && one.emoji.local_only);
    assert_eq!(actions(&one).iter().filter(|action| **action == ReviewAction::REVIEW_ACTION_EDITED).count(), 6);
    let two = ctx.get(Some(&moderator), &second).await.unwrap();
    assert_eq!(two.emoji.aliases, strings(&["only"]));
    assert!(two.emoji.sensitive && !two.emoji.local_only);
    let third = ctx.get(Some(&moderator), &waiting).await.unwrap();
    assert_eq!(
        (third.state, third.emoji.category.as_str()),
        (EmojiState::EMOJI_STATE_PENDING.into(), category.as_str())
    );

    // The catalogue moved for the approved ones only.
    let changed = ctx.changes(version).await;
    let upserted: Vec<&str> = changed.upserted.iter().map(|emoji| emoji.id.as_str()).collect();
    assert!(
        upserted.contains(&first.as_str())
            && upserted.contains(&second.as_str())
            && !upserted.contains(&waiting.as_str())
    );
    assert_eq!(changed.categories.iter().find(|c| c.name == category).unwrap().emoji_count, 2);
    // Removing the category from all of them again.
    assert_eq!(bulk(&ctx, &moderator, &all, Change::SetCategory(String::new())).await, Ok(3));
    assert_eq!(ctx.get(None, &first).await.unwrap().emoji.category, "");

    assert_eq!(bulk(&ctx, &moderator, &[], Change::SetSensitive(true)).await.unwrap_err(), "EMOJI_IDS_INVALID");
    assert_eq!(bulk(&ctx, &moderator, &["nope"], Change::SetSensitive(true)).await.unwrap_err(), "EMOJI_IDS_INVALID");
    let too_many: Vec<String> = (0..201).map(|_| uuid::Uuid::now_v7().to_string()).collect();
    let too_many: Vec<&str> = too_many.iter().map(String::as_str).collect();
    assert_eq!(bulk(&ctx, &moderator, &too_many, Change::SetSensitive(true)).await.unwrap_err(), "EMOJI_IDS_INVALID");
    assert_eq!(
        bulk(&ctx, &moderator, &all, Change::SetCategory(unique("nocategory"))).await.unwrap_err(),
        "CATEGORY_NOT_FOUND"
    );
    assert_eq!(
        bulk(&ctx, &moderator, &all, Change::AddAliases(aliases(&["a b"]))).await.unwrap_err(),
        "ALIASES_INVALID"
    );
    assert_eq!(
        bulk(&ctx, &moderator, &all, Change::SetLicense("x".repeat(1025))).await.unwrap_err(),
        "LICENSE_TOO_LONG"
    );
    let no_change = BulkUpdateEmojiRequest { emoji_ids: strings(&[&first]), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "BulkUpdateEmoji", Some(&moderator), &no_change).await, "CHANGE_REQUIRED");

    // And many are deleted at once.
    let request = BulkDeleteEmojiRequest { emoji_ids: strings(&all), ..Default::default() };
    let deleted: BulkDeleteEmojiResponse =
        ctx.call("Admin", "BulkDeleteEmoji", Some(&moderator), &request).await.unwrap();
    assert_eq!(deleted.deleted_count, 3);
    let gone = ctx.changes(version).await.removed_ids;
    assert!(gone.contains(&first) && gone.contains(&second) && !gone.contains(&waiting));
    for id in [&first, &second, &waiting] {
        assert_eq!(ctx.get(Some(&moderator), id).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    }
    let empty = BulkDeleteEmojiRequest::default();
    assert_eq!(ctx.reason("Admin", "BulkDeleteEmoji", Some(&moderator), &empty).await, "EMOJI_IDS_INVALID");
}

#[tokio::test]
async fn moderators_list_every_emoji_in_any_state() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let category = unique("いちらん");
    ctx.create_category(&moderator, &category).await;
    let token = unique("ls");
    let approved =
        AddEmojiRequest { category: category.clone(), ..ctx.add_request(&moderator, &format!("{token}_ok")).await };
    let approved = ctx.add_with(&moderator, approved).await.emoji.id.clone();
    let pending = ctx.submit(&author, &format!("{token}_wait")).await.emoji.id.clone();
    let rejected = ctx.submit(&author, &format!("{token}_no")).await.emoji.id.clone();
    ctx.reject(&moderator, &rejected, "だめ").await;
    let aliased = AddEmojiRequest {
        aliases: vec![format!("x{token}x")],
        ..ctx.add_request(&moderator, &unique("aliased")).await
    };
    let aliased = ctx.add_with(&moderator, aliased).await.emoji.id.clone();

    let (approved, pending, rejected, aliased) =
        (approved.as_str(), pending.as_str(), rejected.as_str(), aliased.as_str());
    let ids =
        |response: &ListEmojiResponse| response.emojis.iter().map(|detail| detail.emoji.id.clone()).collect::<Vec<_>>();
    // By a part of the name or an alias, newest first, in every state.
    let found = list(&ctx, &moderator, ListEmojiRequest { query: token.to_uppercase(), ..Default::default() }).await;
    assert_eq!(ids(&found), [aliased, rejected, pending, approved]);
    assert!(found.emojis.iter().all(|detail| !detail.history.is_empty()), "with their history");

    let by_state =
        |state: EmojiState| ListEmojiRequest { query: token.clone(), state: state.into(), ..Default::default() };
    assert_eq!(ids(&list(&ctx, &moderator, by_state(EmojiState::EMOJI_STATE_PENDING)).await), [pending]);
    assert_eq!(ids(&list(&ctx, &moderator, by_state(EmojiState::EMOJI_STATE_REJECTED)).await), [rejected]);
    assert_eq!(ids(&list(&ctx, &moderator, by_state(EmojiState::EMOJI_STATE_APPROVED)).await), [aliased, approved]);
    assert!(ids(&list(&ctx, &moderator, by_state(EmojiState::EMOJI_STATE_DISABLED)).await).is_empty());

    let in_category = ListEmojiRequest { category: Some(category.to_uppercase()), ..Default::default() };
    assert_eq!(ids(&list(&ctx, &moderator, in_category).await), [approved]);
    let uncategorized = ListEmojiRequest { query: token.clone(), category: Some(String::new()), ..Default::default() };
    assert_eq!(ids(&list(&ctx, &moderator, uncategorized).await), [aliased, rejected, pending]);
    let by_author = ListEmojiRequest { author_id: author.id.to_string(), ..Default::default() };
    assert_eq!(ids(&list(&ctx, &moderator, by_author).await), [rejected, pending]);

    // Pages.
    let first =
        list(&ctx, &moderator, ListEmojiRequest { query: token.clone(), page: page(3, ""), ..Default::default() })
            .await;
    assert_eq!(ids(&first), [aliased, rejected, pending]);
    assert!(first.page.has_more);
    let rest = ListEmojiRequest { query: token.clone(), page: page(3, &first.page.next_cursor), ..Default::default() };
    let rest = list(&ctx, &moderator, rest).await;
    assert_eq!(ids(&rest), [approved]);
    assert!(!rest.page.has_more);

    let too_long = ListEmojiRequest { query: "q".repeat(65), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "ListEmoji", Some(&moderator), &too_long).await, "QUERY_TOO_LONG");
    let bad_author = ListEmojiRequest { author_id: "nope".to_owned(), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "ListEmoji", Some(&moderator), &bad_author).await, "INVALID_ID");
}
