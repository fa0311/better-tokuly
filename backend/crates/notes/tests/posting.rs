//! The rules agreement and writing notes: posting (with media, stamps,
//! places, hashtags, mentions, replies and quotes), deleting, changing
//! visibility and pinning.
mod common;

use common::*;
use tokuly_identity::settings;
use tokuly_notes::rules::RULES_VERSION;
use tokuly_proto::{
    common::{LatLng, Visibility},
    identity::{UpdateSettingsRequest, UpdateSettingsResponse},
    media::{AssetKind, AssetPurpose},
    notes::*,
    stamp::{CreatePersonalStampRequest, CreatePersonalStampResponse},
};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_testkit::TestApp;
use uuid::Uuid;

#[tokio::test]
async fn rules_are_shown_to_everyone_and_agreed_to_by_version() {
    let ctx = start().await;
    let rules: GetNoteRulesResponse = ctx.note("GetNoteRules", None, &GetNoteRulesRequest::default()).await.unwrap();
    assert_eq!(rules.version, RULES_VERSION);
    assert!(rules.rules.len() >= 5 && rules.rules.iter().all(|rule| !rule.title.is_empty() && !rule.body.is_empty()));
    assert_eq!(rules.rules[0].title, "思いやりを持って投稿しよう!", "the test client asks for Japanese");
    assert!(!rules.agreed && rules.agreed_version == 0 && rules.agreed_at.as_option().is_none());

    let user = ctx.app.user("newcomer").await;
    let mine: GetNoteRulesResponse =
        ctx.note("GetNoteRules", Some(&user), &GetNoteRulesRequest::default()).await.unwrap();
    assert!(!mine.agreed);
    assert_eq!(ctx.post_reason(&user, "hello", |_| {}).await, "RULES_NOT_ACCEPTED");

    let stale = AgreeToNoteRulesRequest { version: RULES_VERSION + 1, ..Default::default() };
    assert_eq!(ctx.note_reason("AgreeToNoteRules", Some(&user), &stale).await, "RULES_VERSION_MISMATCH");
    let current = AgreeToNoteRulesRequest { version: RULES_VERSION, ..Default::default() };
    assert_eq!(ctx.note_reason("AgreeToNoteRules", None, &current).await, "SIGN_IN_REQUIRED");
    let agreed: AgreeToNoteRulesResponse = ctx.note("AgreeToNoteRules", Some(&user), &current).await.unwrap();
    let again: AgreeToNoteRulesResponse = ctx.note("AgreeToNoteRules", Some(&user), &current).await.unwrap();
    assert_eq!(agreed.agreed_at, again.agreed_at, "agreeing twice keeps the first date");

    let mine: GetNoteRulesResponse =
        ctx.note("GetNoteRules", Some(&user), &GetNoteRulesRequest::default()).await.unwrap();
    assert!(mine.agreed && mine.agreed_version == RULES_VERSION && mine.agreed_at.as_option().is_some());
    ctx.post(&user, "hello").await;

    // When the rules change, an old agreement no longer lets anyone post.
    sqlx::query("UPDATE note_rule_agreements SET version = $2 WHERE user_id = $1")
        .bind(user.id)
        .bind(RULES_VERSION - 1)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let mine: GetNoteRulesResponse =
        ctx.note("GetNoteRules", Some(&user), &GetNoteRulesRequest::default()).await.unwrap();
    assert!(!mine.agreed && mine.agreed_version == RULES_VERSION - 1);
    assert_eq!(ctx.post_reason(&user, "again", |_| {}).await, "RULES_NOT_ACCEPTED");
}

#[tokio::test]
async fn a_note_is_posted_with_everything_that_follows_from_it() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    let tag = unique("spring");

    let note = ctx.post(&alice, &format!("  今何してる? #{tag} #Tokuly\r\n散歩中!  ")).await;
    assert_eq!(note.text, format!("今何してる? #{tag} #Tokuly\n散歩中!"), "text is trimmed and line endings unified");
    assert_eq!(note.author.id, alice.id.to_string());
    assert_eq!(note.visibility, Visibility::VISIBILITY_PUBLIC, "the default of the default_note_visibility setting");
    assert_eq!(note.hashtags, vec![tag.clone(), "tokuly".to_owned()]);
    let counters = note.counters.as_option().unwrap();
    assert_eq!(
        (counters.note_id.as_str(), counters.like_count, counters.reply_count, counters.view_count),
        (note.id.as_str(), 0, 0, 0)
    );
    let viewer = note.viewer.as_option().expect("a signed-in caller gets their own state");
    assert!(!viewer.liked && !viewer.renoted && !note.pinned && !note.content_warning);
    assert!(
        note.created_at.as_option().is_some()
            && note.reply_to.as_option().is_none()
            && note.quote.as_option().is_none()
    );

    let note_id = id(&note.id);
    let created = ctx.app.events_by("notes.note.created", alice.id).await;
    assert!(
        created
            .iter()
            .any(|event| event.data["note_id"] == serde_json::json!(note_id) && event.data["visibility"] == 1)
    );
    let activity = ctx.activities_of(alice.id, "note.posted").await;
    assert_eq!(activity.len(), 1);
    assert_eq!(activity[0].data["audience"], "public");
    assert_eq!(activity[0].data["path"], format!("/notes/{note_id}"));
    assert_eq!(activity[0].data["subject"], serde_json::json!([3, note_id]));

    let indexed = ctx.search_changes("note", note_id).await;
    assert_eq!(indexed.len(), 1);
    let doc = &indexed[0].data["doc"];
    assert_eq!(doc["owner_id"], serde_json::json!(alice.id));
    assert_eq!(doc["tags"], serde_json::json!([tag, "tokuly"]));
    assert_eq!(doc["path"], format!("/notes/{note_id}"));
    assert!(doc["public"].as_bool().unwrap());

    // The new hashtag entered the directory and the search index.
    let hashtag_id: Uuid = sqlx::query_scalar("SELECT id FROM hashtags WHERE tag = $1")
        .bind(&tag)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.is_indexed("hashtag", hashtag_id).await);

    assert_eq!(
        ctx.get_note(None, &note.id).await.unwrap().text,
        note.text,
        "public notes are readable without signing in"
    );
    assert!(ctx.get_note(None, &note.id).await.unwrap().viewer.as_option().is_none());
}

#[tokio::test]
async fn posting_is_validated() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);

    assert_eq!(
        ctx.note_reason("CreateNote", None, &CreateNoteRequest { text: "hi".into(), ..Default::default() }).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(ctx.post_reason(&alice, "  \n ", |_| {}).await, "NOTE_EMPTY");
    assert_eq!(ctx.post_reason(&alice, &"あ".repeat(3001), |_| {}).await, "NOTE_TOO_LONG");
    // 3000 "characters" that are each a pile of combining marks are too long as well.
    assert_eq!(
        ctx.post_reason(&alice, &format!("a{}", "\u{301}".repeat(60)).repeat(3000), |_| {}).await,
        "NOTE_TOO_LONG"
    );
    // 3000 user-perceived characters fit, however many bytes they take.
    assert_eq!(ctx.post(&alice, &"👨‍👩‍👧‍👦".repeat(3000)).await.text.chars().count(), 3000 * 7);

    let mut images = Vec::new();
    for _ in 0..11 {
        images.push(ctx.note_image(&alice).await);
    }
    assert_eq!(ctx.post_reason(&alice, "", |r| r.asset_ids = images.clone()).await, "TOO_MANY_ATTACHMENTS");
    let video = ctx.note_video(&alice).await;
    assert_eq!(
        ctx.post_reason(&alice, "", |r| r.asset_ids = vec![video.clone(), images[0].clone()]).await,
        "ATTACHMENTS_MIXED"
    );
    let file = ctx.upload(&alice, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, AssetKind::ASSET_KIND_FILE).await;
    assert_eq!(ctx.post_reason(&alice, "", |r| r.asset_ids = vec![file]).await, "ATTACHMENT_UNSUPPORTED");
    let story_upload = ctx.story_image(&alice).await;
    assert_eq!(ctx.post_reason(&alice, "", |r| r.asset_ids = vec![story_upload]).await, "ASSET_WRONG_PURPOSE");
    let someone_elses = ctx.note_image(&bob).await;
    assert_eq!(ctx.post_reason(&alice, "", |r| r.asset_ids = vec![someone_elses]).await, "ASSET_NOT_FOUND");
    assert_eq!(
        ctx.post_reason(&alice, "", |r| r.asset_ids = vec![Uuid::now_v7().to_string()]).await,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(ctx.post_reason(&alice, "", |r| r.asset_ids = vec!["nope".into()]).await, "INVALID_ID");
    let processing = ctx.note_image(&alice).await;
    sqlx::query("UPDATE media_assets SET status = 2 WHERE id = $1")
        .bind(id(&processing))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.post_reason(&alice, "", |r| r.asset_ids = vec![processing.clone()]).await, "ASSET_NOT_READY");
    sqlx::query("UPDATE media_assets SET status = 4 WHERE id = $1")
        .bind(id(&processing))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.post_reason(&alice, "", |r| r.asset_ids = vec![processing]).await, "ASSET_FAILED");

    assert_eq!(ctx.post_reason(&alice, "hi", |r| r.stamp_id = Uuid::now_v7().to_string()).await, "STAMP_NOT_USABLE");
    assert_eq!(
        ctx.post_reason(&alice, "hi", |r| r.visibility = Visibility::VISIBILITY_UNLISTED.into()).await,
        "INVALID_VISIBILITY"
    );
    assert_eq!(
        ctx.post_reason(&alice, "hi", |r| r.place =
            Some(PlaceTag { name: "nowhere".into(), ..Default::default() }).into())
            .await,
        "INVALID_PLACE"
    );
    let off_the_map = PlaceTag {
        name: "x".into(),
        location: LatLng { lat: 123.0, lng: 0.0, ..Default::default() }.into(),
        ..Default::default()
    };
    assert_eq!(ctx.post_reason(&alice, "hi", |r| r.place = Some(off_the_map).into()).await, "INVALID_PLACE");
    assert_eq!(ctx.post_reason(&alice, "hi", |r| r.idempotency_key = "k".repeat(65)).await, "INVALID_IDEMPOTENCY_KEY");

    assert_eq!(ctx.post_reason(&alice, "hi", |r| r.reply_to_id = Uuid::now_v7().to_string()).await, "NOTE_NOT_FOUND");
    assert_eq!(ctx.post_reason(&alice, "hi", |r| r.quote_of_id = Uuid::now_v7().to_string()).await, "NOTE_NOT_FOUND");
    assert_eq!(ctx.post_reason(&alice, "hi", |r| r.reply_to_id = "nope".into()).await, "INVALID_ID");
    // A note alice cannot see is as good as missing.
    let private = ctx.post_visible_to(&bob, "dear diary", Visibility::VISIBILITY_PRIVATE).await;
    assert_eq!(ctx.post_reason(&alice, "hi", |r| r.reply_to_id = private.id.clone()).await, "NOTE_NOT_FOUND");
    assert_eq!(ctx.post_reason(&alice, "hi", |r| r.quote_of_id = private.id.clone()).await, "NOTE_NOT_FOUND");
    // Only public notes can be quoted, even ones the author can see.
    ctx.befriend(&alice, &bob).await;
    let for_friends = ctx.post_visible_to(&bob, "friends only", Visibility::VISIBILITY_FRIENDS).await;
    assert_eq!(ctx.post_reason(&alice, "look", |r| r.quote_of_id = for_friends.id.clone()).await, "NOTE_NOT_PUBLIC");
}

#[tokio::test]
async fn notes_carry_media_a_stamp_a_place_and_a_content_warning() {
    // This test also needs the stamp domain's API, to make a stamp to send.
    let mut domain = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let stamp =
            Stamp::new(core.clone(), identity.clone(), StampConfig { seed_official: false, ..Default::default() })
                .expect("stamp domain");
        let notes = tokuly_notes::Notes::new(core.clone(), identity.clone(), Default::default()).expect("notes domain");
        let router = notes.register(stamp.register(router));
        domain = Some(notes);
        router
    })
    .await;
    let ctx = Ctx { app, notes: domain.unwrap() };
    let alice = ctx.member("alice").await;

    // Four images, in the order given (an animation counts as an image).
    let mut images = Vec::new();
    for _ in 0..3 {
        images.push(ctx.note_image(&alice).await);
    }
    images.push(ctx.upload(&alice, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, AssetKind::ASSET_KIND_ANIMATION).await);
    let place = PlaceTag {
        name: " 渋谷スクランブル交差点 ".into(),
        location: LatLng { lat: 35.6595, lng: 139.7005, ..Default::default() }.into(),
        ..Default::default()
    };
    let note = ctx
        .post_with(&alice, "", |r| {
            r.asset_ids = images.clone();
            r.place = Some(place).into();
            r.content_warning = true;
        })
        .await;
    assert_eq!(note.attachments.iter().map(|a| a.asset_id.clone()).collect::<Vec<_>>(), images);
    assert!(note.attachments.iter().all(|a| a.image.as_option().is_some_and(|image| !image.url.is_empty())));
    assert!(note.content_warning);
    let tagged = note.place.as_option().unwrap();
    assert_eq!(
        (tagged.name.as_str(), tagged.location.lat, tagged.location.lng),
        ("渋谷スクランブル交差点", 35.6595, 139.7005)
    );
    for image in &images {
        assert!(tokuly_media::assets::is_attached(&ctx.app.core, id(image)).await.unwrap(), "a note holds its uploads");
    }
    let activity = &ctx.activities_of(alice.id, "note.posted").await[0];
    assert_eq!(activity.data["location"], serde_json::json!([35.6595, 139.7005]));
    assert!(activity.data["image_url"].is_null(), "sensitive pictures stay off feed cards");

    // One video, alone.
    let video = ctx.note_video(&alice).await;
    let clip = ctx.post_with(&alice, "見て!", |r| r.asset_ids = vec![video.clone()]).await;
    assert_eq!(clip.attachments.len(), 1);
    assert!(clip.attachments[0].video.as_option().is_some_and(|video| !video.hls_url.is_empty()));

    // A stamp alone is a note; sending it is recorded with the stamp domain.
    let art = ctx.upload(&alice, AssetPurpose::ASSET_PURPOSE_STAMP, AssetKind::ASSET_KIND_IMAGE).await;
    let made: CreatePersonalStampResponse = ctx
        .app
        .call(
            "tokuly.stamp.v1.StampLibraryService/CreatePersonalStamp",
            Some(&alice),
            &CreatePersonalStampRequest { asset_id: art, name: "やったー".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let stamp_id = made.stamp.stamp_ref.id.clone();
    let stamped = ctx.post_with(&alice, "", |r| r.stamp_id = stamp_id.clone()).await;
    assert_eq!(stamped.stamp.as_option().map(|stamp| stamp.id.as_str()), Some(stamp_id.as_str()));
    let sends: i64 =
        sqlx::query_scalar("SELECT count(*) FROM stamp_usage WHERE stamp_id = $1 AND user_id = $2 AND context = 4")
            .bind(id(&stamp_id))
            .bind(alice.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(sends, 1);
    // Someone else's personal stamp cannot be sent.
    let bob = ctx.member("bob").await;
    assert_eq!(ctx.post_reason(&bob, "", |r| r.stamp_id = stamp_id.clone()).await, "STAMP_NOT_USABLE");
}

#[tokio::test]
async fn visibility_defaults_to_the_authors_setting_and_can_be_changed() {
    let ctx = start().await;
    let (alice, friend, stranger) =
        (ctx.member("alice").await, ctx.member("friend").await, ctx.member("stranger").await);
    ctx.befriend(&alice, &friend).await;
    let tag = unique("vis");

    let mut privacy = settings::privacy_of(&ctx.app.core, alice.id).await.unwrap();
    privacy.default_note_visibility = Visibility::VISIBILITY_FRIENDS.into();
    let _: UpdateSettingsResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.AccountService/UpdateSettings",
            Some(&alice),
            &UpdateSettingsRequest { privacy: privacy.into(), ..Default::default() },
        )
        .await
        .unwrap();

    let note = ctx.post(&alice, &format!("for my friends #{tag}")).await;
    assert_eq!(note.visibility, Visibility::VISIBILITY_FRIENDS);
    let note_id = id(&note.id);
    assert!(ctx.search_changes("note", note_id).await.is_empty(), "only public notes are indexed");
    assert_eq!(ctx.activities_of(alice.id, "note.posted").await[0].data["audience"], "friends");
    assert!(ctx.get_note(Some(&friend), &note.id).await.is_ok());
    assert_eq!(ctx.get_note(Some(&stranger), &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    assert_eq!(ctx.get_note(None, &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    let count = |ctx: &Ctx, tag: String| {
        let db = ctx.app.core.db.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT note_count FROM hashtags WHERE tag = $1")
                .bind(tag)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(count(&ctx, tag.clone()).await, 0, "hashtag counts are about public notes");

    let change = |visibility: Visibility| SetNoteVisibilityRequest {
        note_id: note.id.clone(),
        visibility: visibility.into(),
        ..Default::default()
    };
    assert_eq!(
        ctx.note_reason("SetNoteVisibility", Some(&alice), &change(Visibility::VISIBILITY_UNSPECIFIED)).await,
        "INVALID_VISIBILITY"
    );
    assert_eq!(
        ctx.note_reason("SetNoteVisibility", Some(&alice), &change(Visibility::VISIBILITY_NO_FEED)).await,
        "INVALID_VISIBILITY"
    );
    assert_eq!(
        ctx.note_reason("SetNoteVisibility", Some(&friend), &change(Visibility::VISIBILITY_PUBLIC)).await,
        "NOT_NOTE_AUTHOR"
    );
    assert_eq!(
        ctx.note_reason("SetNoteVisibility", Some(&stranger), &change(Visibility::VISIBILITY_PUBLIC)).await,
        "NOTE_NOT_FOUND"
    );
    assert_eq!(
        ctx.note_reason("SetNoteVisibility", None, &change(Visibility::VISIBILITY_PUBLIC)).await,
        "SIGN_IN_REQUIRED"
    );

    let public: SetNoteVisibilityResponse =
        ctx.note("SetNoteVisibility", Some(&alice), &change(Visibility::VISIBILITY_PUBLIC)).await.unwrap();
    assert_eq!(public.note.visibility, Visibility::VISIBILITY_PUBLIC);
    assert!(ctx.get_note(Some(&stranger), &note.id).await.is_ok());
    assert!(ctx.is_indexed("note", note_id).await);
    assert_eq!(count(&ctx, tag.clone()).await, 1);
    let events = ctx.app.events_by("notes.note.visibility_changed", alice.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["visibility"], 1);

    let private: SetNoteVisibilityResponse =
        ctx.note("SetNoteVisibility", Some(&alice), &change(Visibility::VISIBILITY_PRIVATE)).await.unwrap();
    assert_eq!(private.note.visibility, Visibility::VISIBILITY_PRIVATE);
    assert_eq!(ctx.get_note(Some(&friend), &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    assert!(ctx.get_note(Some(&alice), &note.id).await.is_ok(), "the author always sees their own notes");
    assert!(!ctx.is_indexed("note", note_id).await, "a note that stopped being public leaves the index");
    assert_eq!(count(&ctx, tag).await, 0);
    // Setting the same visibility again is not a change.
    let _: SetNoteVisibilityResponse =
        ctx.note("SetNoteVisibility", Some(&alice), &change(Visibility::VISIBILITY_PRIVATE)).await.unwrap();
    assert_eq!(ctx.app.events_by("notes.note.visibility_changed", alice.id).await.len(), 2);
}

#[tokio::test]
async fn replies_and_quotes_link_notes_and_notify_their_authors() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    ctx.befriend(&alice, &bob).await;
    let original = ctx.post(&alice, "original").await;

    let reply = ctx.reply(&bob, &original, "a reply").await;
    let target = reply.reply_to.as_option().unwrap();
    assert_eq!(
        (target.note_id.as_str(), target.author.id.clone(), target.unavailable),
        (original.id.as_str(), alice.id.to_string(), false)
    );
    assert_eq!(ctx.get_note(Some(&alice), &original.id).await.unwrap().counters.reply_count, 1);
    let notified = ctx.notifications_for(alice.id, "notes.reply").await;
    assert_eq!(notified.len(), 1);
    assert_eq!(notified[0].data["actor_id"], serde_json::json!(bob.id));
    assert_eq!(notified[0].data["path"], format!("/notes/{}", reply.id));
    assert_eq!(notified[0].data["body"]["ja"], "a reply");
    assert!(notified[0].data["title"]["ja"].as_str().unwrap().contains("返信"));
    assert_eq!(ctx.activities_of(bob.id, "note.replied").await.len(), 1);
    assert_eq!(ctx.activities_of(bob.id, "note.replied").await[0].data["with_user_ids"], serde_json::json!([alice.id]));
    // Replying to a friend feeds the best-friend ranking.
    let interactions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM friend_interaction_days WHERE user_lo = LEAST($1, $2) AND user_hi = GREATEST($1, $2)",
    )
    .bind(alice.id)
    .bind(bob.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(interactions, 1);
    // Answering yourself notifies nobody.
    ctx.reply(&alice, &original, "replying to myself").await;
    assert_eq!(ctx.notifications_for(alice.id, "notes.reply").await.len(), 1);
    // Neither does a private reply: nobody is told about a note they cannot open.
    ctx.post_with(&carol, "a note to myself", |r| {
        r.reply_to_id = original.id.clone();
        r.visibility = Visibility::VISIBILITY_PRIVATE.into();
    })
    .await;
    assert_eq!(ctx.notifications_for(alice.id, "notes.reply").await.len(), 1);

    let quote = ctx.quote(&carol, &original, "so true").await;
    let quoted = quote.quote.as_option().unwrap();
    assert!(!quoted.unavailable);
    assert_eq!(quoted.note.text, "original");
    assert!(quoted.note.quote.as_option().is_none(), "a quote is rendered one level deep");
    assert_eq!(ctx.get_note(None, &original.id).await.unwrap().counters.quote_count, 1);
    assert_eq!(ctx.notifications_for(alice.id, "notes.quote").await.len(), 1);

    // A reply is never shown more widely than the note it answers.
    let for_friends = ctx.post_visible_to(&alice, "friends only", Visibility::VISIBILITY_FRIENDS).await;
    let narrowed = ctx
        .post_with(&bob, "public reply?", |r| {
            r.reply_to_id = for_friends.id.clone();
            r.visibility = Visibility::VISIBILITY_PUBLIC.into();
        })
        .await;
    assert_eq!(narrowed.visibility, Visibility::VISIBILITY_FRIENDS);
    assert_eq!(ctx.get_note(Some(&carol), &narrowed.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    // …and it cannot be widened afterwards either.
    let widen = SetNoteVisibilityRequest {
        note_id: narrowed.id.clone(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        ..Default::default()
    };
    let still: SetNoteVisibilityResponse = ctx.note("SetNoteVisibility", Some(&bob), &widen).await.unwrap();
    assert_eq!(still.note.visibility, Visibility::VISIBILITY_FRIENDS);
    // A friends-only reply reaches the author it answers even when the two are not friends.
    let public = ctx.post(&alice, "anyone?").await;
    let quiet = ctx
        .post_with(&carol, "just for you", |r| {
            r.reply_to_id = public.id.clone();
            r.visibility = Visibility::VISIBILITY_FRIENDS.into();
        })
        .await;
    assert!(ctx.get_note(Some(&alice), &quiet.id).await.is_ok());
    assert_eq!(ctx.get_note(Some(&bob), &quiet.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");

    // Deleting a note keeps its replies; they show what they answered as unavailable.
    ctx.delete_note(&alice, &original).await;
    let orphan = ctx.get_note(Some(&bob), &reply.id).await.unwrap();
    let target = orphan.reply_to.as_option().unwrap();
    assert!(target.unavailable && target.author.id == alice.id.to_string());
    let quote = ctx.get_note(Some(&carol), &quote.id).await.unwrap();
    assert!(quote.quote.unavailable && quote.quote.note.as_option().is_none());
}

#[tokio::test]
async fn a_reply_does_not_name_whom_it_answers_across_a_block() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    let original = ctx.post(&alice, "alice's note").await;
    let reply = ctx.reply(&carol, &original, "carol's reply").await;
    ctx.block(&bob, &alice).await;

    // Bob reads carol's reply; the note it answers, and its author, do not exist for him.
    let for_bob = ctx.get_note(Some(&bob), &reply.id).await.unwrap();
    let target = for_bob.reply_to.as_option().unwrap();
    assert!(target.unavailable && target.author.as_option().is_none());
    assert_eq!(target.note_id, original.id);
    // Everyone else still sees whom carol answered.
    let for_anyone = ctx.get_note(None, &reply.id).await.unwrap();
    assert!(!for_anyone.reply_to.unavailable && for_anyone.reply_to.author.id == alice.id.to_string());
}

#[tokio::test]
async fn mentions_are_resolved_stored_and_notified_to_people_who_may_see_the_note() {
    let ctx = start().await;
    let (alice, bob, carol, dave) =
        (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await, ctx.member("dave").await);
    ctx.befriend(&alice, &bob).await;
    ctx.block(&dave, &alice).await;

    let text = format!(
        "@{} と@{} と @{} と @nobody_here_{} と @{} #会った",
        bob.handle.to_uppercase(),
        carol.handle,
        dave.handle,
        unique(""),
        alice.handle
    );
    let note = ctx.post(&alice, &text).await;
    let mentioned: Vec<(&str, &str)> = note.mentions.iter().map(|m| (m.handle.as_str(), m.user_id.as_str())).collect();
    // In order of appearance; unknown handles and people in a block with the author stay plain text.
    assert_eq!(
        mentioned,
        vec![
            (bob.handle.as_str(), &*bob.id.to_string()),
            (carol.handle.as_str(), &*carol.id.to_string()),
            (alice.handle.as_str(), &*alice.id.to_string())
        ]
    );
    assert_eq!(ctx.notifications_for(bob.id, "notes.mention").await.len(), 1);
    assert_eq!(ctx.notifications_for(carol.id, "notes.mention").await.len(), 1);
    assert!(ctx.notifications_for(dave.id, "notes.mention").await.is_empty());
    assert!(ctx.notifications_for(alice.id, "notes.mention").await.is_empty(), "mentioning yourself notifies nobody");

    // A friends-only note only notifies the mentioned people who can read it.
    let private_text = format!("@{} @{} ひみつ", bob.handle, carol.handle);
    ctx.post_visible_to(&alice, &private_text, Visibility::VISIBILITY_FRIENDS).await;
    assert_eq!(ctx.notifications_for(bob.id, "notes.mention").await.len(), 2);
    assert_eq!(ctx.notifications_for(carol.id, "notes.mention").await.len(), 1);

    // Someone answered and mentioned in the same note hears about it once.
    let original = ctx.post(&bob, "bob's note").await;
    ctx.reply(&alice, &original, &format!("@{} はい!", bob.handle)).await;
    assert_eq!(ctx.notifications_for(bob.id, "notes.reply").await.len(), 1);
    assert_eq!(ctx.notifications_for(bob.id, "notes.mention").await.len(), 2);
}

#[tokio::test]
async fn retried_posts_with_the_same_key_create_one_note() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    let key = unique("key");
    let first = ctx.post_with(&alice, "once", |r| r.idempotency_key = key.clone()).await;
    let second = ctx.post_with(&alice, "once", |r| r.idempotency_key = key.clone()).await;
    assert_eq!(first.id, second.id);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM notes WHERE author_id = $1")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(count, 1);
    // A request that failed validation does not burn its key.
    let other = unique("key");
    assert_eq!(ctx.post_reason(&alice, "", |r| r.idempotency_key = other.clone()).await, "NOTE_EMPTY");
    ctx.post_with(&alice, "fixed", |r| r.idempotency_key = other.clone()).await;
    // Keys are per user.
    let bob = ctx.member("bob").await;
    assert_ne!(ctx.post_with(&bob, "once", |r| r.idempotency_key = key.clone()).await.id, first.id);
}

#[tokio::test]
async fn posting_is_rate_limited() {
    let ctx = start_with(tokuly_notes::NotesConfig { posts_per_minute: 3, ..Default::default() }).await;
    let alice = ctx.member("alice").await;
    for n in 0..3 {
        ctx.post(&alice, &format!("note {n}")).await;
    }
    assert_eq!(ctx.post_reason(&alice, "one too many", |_| {}).await, "RATE_LIMITED");
}

#[tokio::test]
async fn notes_are_deleted_by_their_author_only() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let tag = unique("gone");
    let image = ctx.note_image(&alice).await;
    let note = ctx.post_with(&alice, &format!("bye #{tag}"), |r| r.asset_ids = vec![image.clone()]).await;
    let note_id = id(&note.id);
    ctx.like(&bob, &note).await;
    ctx.renote(&bob, &note).await;
    let hidden = ctx.post_visible_to(&alice, "private", Visibility::VISIBILITY_PRIVATE).await;

    let request = DeleteNoteRequest { note_id: note.id.clone(), ..Default::default() };
    assert_eq!(ctx.note_reason("DeleteNote", None, &request).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.note_reason("DeleteNote", Some(&bob), &request).await, "NOT_NOTE_AUTHOR");
    let invisible = DeleteNoteRequest { note_id: hidden.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.note_reason("DeleteNote", Some(&bob), &invisible).await,
        "NOTE_NOT_FOUND",
        "a hidden note does not admit to existing"
    );

    let mut watching = ctx.app.listen_topic(&format!("subject:3:{note_id}")).await;
    ctx.delete_note(&alice, &note).await;
    assert_eq!(next_push(&mut watching).await, Some(notes_event::Event::NoteDeletedId(note.id.clone())));
    assert_eq!(ctx.get_note(Some(&alice), &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    assert_eq!(ctx.note_reason("DeleteNote", Some(&alice), &request).await, "NOTE_NOT_FOUND");

    let db = &ctx.app.core.db;
    let leftovers: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM note_likes WHERE note_id = $1) + (SELECT count(*) FROM note_renotes WHERE note_id = $1)
              + (SELECT count(*) FROM note_hashtags WHERE note_id = $1)",
    )
    .bind(note_id)
    .fetch_one(db)
    .await
    .unwrap();
    assert_eq!(leftovers, 0);
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, id(&image)).await.unwrap(), "the upload is released");
    assert!(!ctx.is_indexed("note", note_id).await);
    let (hashtag_id, count): (Uuid, i64) =
        sqlx::query_as("SELECT id, note_count FROM hashtags WHERE tag = $1").bind(&tag).fetch_one(db).await.unwrap();
    assert_eq!(count, 0);
    assert!(!ctx.is_indexed("hashtag", hashtag_id).await, "a hashtag without public notes leaves the index");
    let deleted = ctx.app.events_by("notes.note.deleted", alice.id).await;
    assert_eq!(deleted.len(), 1);
    assert!(ctx.app.subject_deleted(tokuly_proto::common::SubjectType::SUBJECT_TYPE_NOTE, &note_id.to_string()).await);
    assert_eq!(
        (deleted[0].data["note_id"].clone(), deleted[0].data["reason"].clone()),
        (serde_json::json!(note_id), serde_json::json!("author"))
    );
}

#[tokio::test]
async fn one_note_is_pinned_to_a_profile() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let first = ctx.post(&alice, "first").await;
    let second = ctx.post(&alice, "second").await;
    let pin = |note: &Note| PinNoteRequest { note_id: note.id.clone(), ..Default::default() };
    let timeline = |viewer: Option<tokuly_testkit::TestUser>| {
        let request = GetUserTimelineRequest { user_id: alice.id.to_string(), ..Default::default() };
        let ctx = &ctx;
        async move { ctx.note::<_, GetUserTimelineResponse>("GetUserTimeline", viewer.as_ref(), &request).await.unwrap() }
    };

    assert_eq!(ctx.note_reason("PinNote", None, &pin(&first)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.note_reason("PinNote", Some(&bob), &pin(&first)).await, "NOT_NOTE_AUTHOR");
    assert_eq!(
        ctx.note_reason(
            "PinNote",
            Some(&alice),
            &PinNoteRequest { note_id: Uuid::now_v7().to_string(), ..Default::default() }
        )
        .await,
        "NOTE_NOT_FOUND"
    );

    let pinned_ids =
        |page: &GetUserTimelineResponse| page.pinned_notes.iter().map(|note| note.id.clone()).collect::<Vec<_>>();
    let _: PinNoteResponse = ctx.note("PinNote", Some(&alice), &pin(&first)).await.unwrap();
    assert!(ctx.get_note(None, &first.id).await.unwrap().pinned);
    assert_eq!(pinned_ids(&timeline(None).await), vec![first.id.clone()]);
    // Another pin goes on top; pinning the same note twice is fine.
    let _: PinNoteResponse = ctx.note("PinNote", Some(&alice), &pin(&second)).await.unwrap();
    let _: PinNoteResponse = ctx.note("PinNote", Some(&alice), &pin(&second)).await.unwrap();
    let page = timeline(Some(bob.clone())).await;
    assert_eq!(pinned_ids(&page), vec![second.id.clone(), first.id.clone()]);
    assert_eq!(texts(&page.entries), vec!["second", "first"], "pinned notes also stay in their place in the list");

    // Up to five, also when they are pinned at once.
    let mut more = Vec::new();
    for index in 0..4 {
        more.push(ctx.post(&alice, &format!("more {index}")).await);
    }
    let requests: Vec<PinNoteRequest> = more.iter().map(pin).collect();
    let results = futures::future::join_all(
        requests.iter().map(|request| ctx.note::<_, PinNoteResponse>("PinNote", Some(&alice), request)),
    )
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 3);
    assert!(results.iter().filter_map(|result| result.as_ref().err()).all(|e| e.reason == "TOO_MANY_PINNED"));
    assert_eq!(timeline(Some(alice.clone())).await.pinned_notes.len(), 5);

    // A pinned note only shows to people who may see it.
    let hide = SetNoteVisibilityRequest {
        note_id: second.id.clone(),
        visibility: Visibility::VISIBILITY_PRIVATE.into(),
        ..Default::default()
    };
    let _: SetNoteVisibilityResponse = ctx.note("SetNoteVisibility", Some(&alice), &hide).await.unwrap();
    assert!(!pinned_ids(&timeline(Some(bob.clone())).await).contains(&second.id));
    assert!(pinned_ids(&timeline(Some(alice.clone())).await).contains(&second.id));

    let unpin = UnpinNoteRequest { note_id: second.id.clone(), ..Default::default() };
    assert_eq!(ctx.note_reason("UnpinNote", None, &unpin).await, "SIGN_IN_REQUIRED");
    let _: UnpinNoteResponse = ctx.note("UnpinNote", Some(&alice), &unpin).await.unwrap();
    let _: UnpinNoteResponse = ctx.note("UnpinNote", Some(&alice), &unpin).await.unwrap();
    assert!(!pinned_ids(&timeline(Some(alice.clone())).await).contains(&second.id));
    assert!(!ctx.get_note(Some(&alice), &second.id).await.unwrap().pinned);
}

#[tokio::test]
async fn concurrent_visibility_changes_count_a_note_once() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let tag = unique("tag");
    let note = ctx.post(&alice, &format!("mine #{tag}")).await;
    ctx.post(&bob, &format!("someone else's #{tag}")).await;
    let count = || {
        let (db, tag) = (ctx.app.core.db.clone(), tag.clone());
        async move {
            sqlx::query_scalar::<_, i64>("SELECT note_count FROM hashtags WHERE tag = $1")
                .bind(tag)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    let change = |visibility: Visibility| {
        let request =
            SetNoteVisibilityRequest { note_id: note.id.clone(), visibility: visibility.into(), ..Default::default() };
        let (ctx, alice) = (&ctx, alice.clone());
        async move {
            let _: SetNoteVisibilityResponse = ctx.note("SetNoteVisibility", Some(&alice), &request).await.unwrap();
        }
    };
    assert_eq!(count().await, 2);
    // Two rounds of twelve: within the per-minute limit of visibility changes.
    for _ in 0..2 {
        // Several narrowings of the same public note at once take it out of the count once.
        futures::future::join_all(
            [Visibility::VISIBILITY_FRIENDS, Visibility::VISIBILITY_PRIVATE].into_iter().cycle().take(6).map(change),
        )
        .await;
        assert_eq!(count().await, 1, "only bob's note is public");
        futures::future::join_all(std::iter::repeat_n(Visibility::VISIBILITY_PUBLIC, 6).map(change)).await;
        assert_eq!(count().await, 2, "and making it public again counts it once");
    }
}

#[tokio::test]
async fn a_narrowed_note_says_who_can_still_see_it() {
    let ctx = start().await;
    let (alice, friend, parent_author, stranger) = (
        ctx.member("alice").await,
        ctx.member("friend").await,
        ctx.member("parent").await,
        ctx.member("stranger").await,
    );
    ctx.befriend(&alice, &friend).await;
    let parent = ctx.post(&parent_author, "a public question").await;
    let reply = ctx
        .post_with(&alice, &format!("@{} @{} answer", stranger.handle, friend.handle), |r| {
            r.reply_to_id = parent.id.clone();
        })
        .await;
    assert_eq!(ctx.notifications_for(stranger.id, "notes.mention").await.len(), 1, "the stranger was told");

    let change = |visibility: Visibility| SetNoteVisibilityRequest {
        note_id: reply.id.clone(),
        visibility: visibility.into(),
        ..Default::default()
    };
    let _: SetNoteVisibilityResponse =
        ctx.note("SetNoteVisibility", Some(&alice), &change(Visibility::VISIBILITY_FRIENDS)).await.unwrap();
    let _: SetNoteVisibilityResponse =
        ctx.note("SetNoteVisibility", Some(&alice), &change(Visibility::VISIBILITY_PRIVATE)).await.unwrap();
    let _: SetNoteVisibilityResponse =
        ctx.note("SetNoteVisibility", Some(&alice), &change(Visibility::VISIBILITY_PUBLIC)).await.unwrap();
    let events = ctx.app.events_by("notes.note.visibility_changed", alice.id).await;
    assert_eq!(events.len(), 3);
    // Friends only: the author's friends and the person answered may still
    // see it, so the notification domain can withdraw what reached anyone else.
    let mut visible_to: Vec<String> = serde_json::from_value(events[0].data["visible_to"].clone()).unwrap();
    visible_to.sort();
    let mut expected = vec![friend.id.to_string(), parent_author.id.to_string()];
    expected.sort();
    assert_eq!(visible_to, expected);
    assert_eq!(events[1].data["visible_to"], serde_json::json!([]), "private: nobody else");
    assert!(events[2].data["visible_to"].is_null(), "public: everyone");
}

#[tokio::test]
async fn visibility_changes_are_rate_limited_and_carry_when_they_were_made() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    let note = ctx.post(&alice, "flip").await;
    let before = chrono::Utc::now();
    for round in 0..30 {
        let visibility = if round % 2 == 0 { Visibility::VISIBILITY_FRIENDS } else { Visibility::VISIBILITY_PUBLIC };
        let request =
            SetNoteVisibilityRequest { note_id: note.id.clone(), visibility: visibility.into(), ..Default::default() };
        let _: SetNoteVisibilityResponse = ctx.note("SetNoteVisibility", Some(&alice), &request).await.unwrap();
    }
    let request = SetNoteVisibilityRequest {
        note_id: note.id.clone(),
        visibility: Visibility::VISIBILITY_PRIVATE.into(),
        ..Default::default()
    };
    assert_eq!(ctx.note_reason("SetNoteVisibility", Some(&alice), &request).await, "RATE_LIMITED");

    let events = ctx.app.events_by("notes.note.visibility_changed", alice.id).await;
    assert_eq!(events.len(), 30);
    let times: Vec<chrono::DateTime<chrono::Utc>> =
        events.iter().map(|event| serde_json::from_value(event.data["changed_at"].clone()).unwrap()).collect();
    assert!(times[0] >= before && times.windows(2).all(|pair| pair[0] < pair[1]), "in the order they were made");
}
