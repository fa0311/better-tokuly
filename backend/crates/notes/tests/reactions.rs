//! Likes, renotes, the lists of people behind them, and view counting.
mod common;

use common::*;
use tokuly_proto::{
    common::Visibility,
    notes::{notes_event::Event as Push, *},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn like_of(note_id: &str) -> LikeNoteRequest {
    LikeNoteRequest { note_id: note_id.to_owned(), ..Default::default() }
}

fn unlike_of(note_id: &str) -> UnlikeNoteRequest {
    UnlikeNoteRequest { note_id: note_id.to_owned(), ..Default::default() }
}

async fn unlike(ctx: &Ctx, user: &TestUser, note: &Note) -> NoteCounters {
    let response: UnlikeNoteResponse = ctx.note("UnlikeNote", Some(user), &unlike_of(&note.id)).await.expect("unlike");
    response.counters.as_option().cloned().unwrap_or_default()
}

async fn view_count(ctx: &Ctx, note: &Note) -> i64 {
    ctx.get_note(None, &note.id).await.unwrap().counters.view_count
}

async fn record_views(ctx: &Ctx, viewer: Option<&TestUser>, notes: &[&Note]) {
    let request =
        RecordNoteViewsRequest { note_ids: notes.iter().map(|note| note.id.clone()).collect(), ..Default::default() };
    let _: RecordNoteViewsResponse = ctx.note("RecordNoteViews", viewer, &request).await.expect("record views");
}

#[tokio::test]
async fn likes_count_once_update_live_and_notify_the_author() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    ctx.befriend(&alice, &bob).await;
    let note = ctx.post(&alice, "like me").await;
    let mut watching = ctx.app.listen_topic(&format!("subject:3:{}", note.id)).await;

    assert_eq!(ctx.like(&bob, &note).await.like_count, 1);
    match next_push(&mut watching).await {
        Some(Push::CountersChanged(counters)) => {
            assert_eq!((counters.note_id.as_str(), counters.like_count), (note.id.as_str(), 1))
        }
        other => panic!("expected live counters, got {other:?}"),
    }
    assert_eq!(ctx.like(&bob, &note).await.like_count, 1, "liking twice changes nothing");
    assert_eq!(ctx.like(&carol, &note).await.like_count, 2);
    assert!(ctx.get_note(Some(&bob), &note.id).await.unwrap().viewer.liked);
    assert!(!ctx.get_note(Some(&alice), &note.id).await.unwrap().viewer.liked);

    let notified = ctx.notifications_for(alice.id, "notes.like").await;
    assert_eq!(notified.len(), 2);
    assert!(
        notified.iter().all(|event| event.data["collapse_key"] == format!("notes.like:{}", note.id)),
        "likes of one note collapse"
    );
    assert_eq!(notified[0].data["actor_id"], serde_json::json!(bob.id));
    assert_eq!(notified[0].data["body"]["ja"], "like me");
    assert_eq!(notified[0].data["subject"], serde_json::json!([3, id(&note.id)]));
    // A like between friends feeds the best-friend ranking; one between strangers does not.
    let interactions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM friend_interaction_days WHERE $1 IN (user_lo, user_hi)")
            .bind(alice.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(interactions, 1);

    assert_eq!(unlike(&ctx, &bob, &note).await.like_count, 1);
    assert_eq!(unlike(&ctx, &bob, &note).await.like_count, 1, "taking back a like that is not there succeeds");
    assert!(!ctx.get_note(Some(&bob), &note.id).await.unwrap().viewer.liked);
    // Liking again right away is not announced again.
    assert_eq!(ctx.like(&bob, &note).await.like_count, 2);
    assert_eq!(ctx.notifications_for(alice.id, "notes.like").await.len(), 2);
    // Liking your own note counts and notifies nobody.
    assert_eq!(ctx.like(&alice, &note).await.like_count, 3);
    assert_eq!(ctx.notifications_for(alice.id, "notes.like").await.len(), 2);

    assert_eq!(ctx.note_reason("LikeNote", None, &like_of(&note.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.note_reason("UnlikeNote", None, &unlike_of(&note.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.note_reason("LikeNote", Some(&bob), &like_of(&Uuid::now_v7().to_string())).await, "NOTE_NOT_FOUND");
    assert_eq!(
        ctx.note_reason("UnlikeNote", Some(&bob), &unlike_of(&Uuid::now_v7().to_string())).await,
        "NOTE_NOT_FOUND"
    );
    assert_eq!(ctx.note_reason("LikeNote", Some(&bob), &like_of("nope")).await, "INVALID_ID");
    let private = ctx.post_visible_to(&alice, "mine", Visibility::VISIBILITY_PRIVATE).await;
    assert_eq!(ctx.note_reason("LikeNote", Some(&bob), &like_of(&private.id)).await, "NOTE_NOT_FOUND");
    assert_eq!(ctx.note_reason("UnlikeNote", Some(&bob), &unlike_of(&private.id)).await, "NOTE_NOT_FOUND");
}

#[tokio::test]
async fn a_like_can_be_taken_back_from_a_note_that_went_private() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let note = ctx.post(&alice, "soon private").await;
    ctx.like(&bob, &note).await;
    let hide = SetNoteVisibilityRequest {
        note_id: note.id.clone(),
        visibility: Visibility::VISIBILITY_PRIVATE.into(),
        ..Default::default()
    };
    let _: SetNoteVisibilityResponse = ctx.note("SetNoteVisibility", Some(&alice), &hide).await.unwrap();

    // The like goes; the numbers of a note bob may no longer see are not returned.
    assert_eq!(unlike(&ctx, &bob, &note).await, NoteCounters::default());
    assert_eq!(ctx.get_note(Some(&alice), &note.id).await.unwrap().counters.like_count, 0);
    assert_eq!(ctx.note_reason("UnlikeNote", Some(&bob), &unlike_of(&note.id)).await, "NOTE_NOT_FOUND");
}

#[tokio::test]
async fn renotes_are_one_per_user_undoable_and_only_for_public_notes() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    ctx.befriend(&alice, &bob).await;
    let note = ctx.post(&alice, "renote me").await;
    let renote_of = |note_id: &str| RenoteNoteRequest { note_id: note_id.to_owned(), ..Default::default() };
    let undo_of = |note_id: &str| UndoRenoteRequest { note_id: note_id.to_owned(), ..Default::default() };

    assert_eq!(ctx.renote(&bob, &note).await.renote_count, 1);
    assert_eq!(ctx.renote(&bob, &note).await.renote_count, 1, "one renote per user per note");
    assert!(ctx.get_note(Some(&bob), &note.id).await.unwrap().viewer.renoted);
    let notified = ctx.notifications_for(alice.id, "notes.renote").await;
    assert_eq!(notified.len(), 1);
    assert_eq!(notified[0].data["actor_id"], serde_json::json!(bob.id));
    assert!(notified[0].data["title"]["ja"].as_str().unwrap().contains("リノート"));

    let undone: UndoRenoteResponse = ctx.note("UndoRenote", Some(&bob), &undo_of(&note.id)).await.unwrap();
    assert_eq!(undone.counters.renote_count, 0);
    let again: UndoRenoteResponse = ctx.note("UndoRenote", Some(&bob), &undo_of(&note.id)).await.unwrap();
    assert_eq!(again.counters.renote_count, 0);
    assert!(!ctx.get_note(Some(&bob), &note.id).await.unwrap().viewer.renoted);

    // Other people pass on only notes everyone may see.
    let for_friends = ctx.post_visible_to(&alice, "friends only", Visibility::VISIBILITY_FRIENDS).await;
    assert_eq!(ctx.note_reason("RenoteNote", Some(&bob), &renote_of(&for_friends.id)).await, "NOTE_NOT_RENOTABLE");
    let private = ctx.post_visible_to(&alice, "mine", Visibility::VISIBILITY_PRIVATE).await;
    assert_eq!(ctx.note_reason("RenoteNote", Some(&bob), &renote_of(&private.id)).await, "NOTE_NOT_FOUND");
    assert_eq!(ctx.note_reason("RenoteNote", None, &renote_of(&note.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.note_reason("UndoRenote", None, &undo_of(&note.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(
        ctx.note_reason("UndoRenote", Some(&bob), &undo_of(&Uuid::now_v7().to_string())).await,
        "NOTE_NOT_FOUND"
    );
}

#[tokio::test]
async fn likers_and_renoters_are_listed_newest_first_without_blocked_people() {
    let ctx = start().await;
    let (alice, bob, carol, dave) =
        (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await, ctx.member("dave").await);
    let note = ctx.post(&alice, "popular").await;
    for user in [&bob, &carol, &dave] {
        ctx.like(user, &note).await;
        ctx.renote(user, &note).await;
    }
    let likers = |viewer: Option<TestUser>, cursor: String| {
        let request = ListNoteLikersRequest { note_id: note.id.clone(), page: page(2, &cursor), ..Default::default() };
        let ctx = &ctx;
        async move { ctx.note::<_, ListNoteLikersResponse>("ListNoteLikers", viewer.as_ref(), &request).await.unwrap() }
    };
    let ids =
        |users: &[tokuly_proto::common::UserSummary]| users.iter().map(|user| user.id.clone()).collect::<Vec<_>>();

    let first = likers(None, String::new()).await;
    assert_eq!(ids(&first.users), vec![dave.id.to_string(), carol.id.to_string()]);
    assert!(first.page.has_more);
    let second = likers(None, first.page.next_cursor.clone()).await;
    assert_eq!(ids(&second.users), vec![bob.id.to_string()]);
    assert!(!second.page.has_more);

    let renoters: ListNoteRenotersResponse = ctx
        .note(
            "ListNoteRenoters",
            Some(&alice),
            &ListNoteRenotersRequest { note_id: note.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(ids(&renoters.users), vec![dave.id.to_string(), carol.id.to_string(), bob.id.to_string()]);

    // A block hides the two from each other in these lists too.
    ctx.block(&carol, &bob).await;
    let seen_by_bob = likers(Some(bob.clone()), String::new()).await;
    assert_eq!(ids(&seen_by_bob.users), vec![dave.id.to_string(), bob.id.to_string()]);
    let seen_by_carol: ListNoteRenotersResponse = ctx
        .note(
            "ListNoteRenoters",
            Some(&carol),
            &ListNoteRenotersRequest { note_id: note.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(ids(&seen_by_carol.users), vec![dave.id.to_string(), carol.id.to_string()]);

    let private = ctx.post_visible_to(&alice, "mine", Visibility::VISIBILITY_PRIVATE).await;
    let hidden = ListNoteLikersRequest { note_id: private.id.clone(), ..Default::default() };
    assert_eq!(ctx.note_reason("ListNoteLikers", Some(&bob), &hidden).await, "NOTE_NOT_FOUND");
    let hidden = ListNoteRenotersRequest { note_id: private.id.clone(), ..Default::default() };
    assert_eq!(ctx.note_reason("ListNoteRenoters", None, &hidden).await, "NOTE_NOT_FOUND");
    let bad_cursor = ListNoteLikersRequest { note_id: note.id.clone(), page: page(2, "!!!"), ..Default::default() };
    assert_eq!(ctx.note_reason("ListNoteLikers", None, &bad_cursor).await, "INVALID_CURSOR");
}

#[tokio::test]
async fn views_count_once_per_viewer_and_never_for_the_author() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    let note = ctx.post(&alice, "look").await;
    let other = ctx.post(&alice, "and here").await;
    let private = ctx.post_visible_to(&alice, "mine", Visibility::VISIBILITY_PRIVATE).await;

    record_views(&ctx, Some(&bob), &[&note, &other, &private]).await;
    assert_eq!((view_count(&ctx, &note).await, view_count(&ctx, &other).await), (1, 1));
    record_views(&ctx, Some(&bob), &[&note, &note]).await;
    assert_eq!(view_count(&ctx, &note).await, 1, "the same viewer counts once within the window");
    record_views(&ctx, Some(&alice), &[&note]).await;
    assert_eq!(view_count(&ctx, &note).await, 1, "authors do not count towards their own notes");
    record_views(&ctx, Some(&carol), &[&note]).await;
    assert_eq!(view_count(&ctx, &note).await, 2);
    // Anonymous viewers count once per address.
    record_views(&ctx, None, &[&note]).await;
    record_views(&ctx, None, &[&note]).await;
    assert_eq!(view_count(&ctx, &note).await, 3);
    // A note the viewer may not see is not counted.
    let hidden_views: i64 = sqlx::query_scalar("SELECT view_count FROM notes WHERE id = $1")
        .bind(id(&private.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(hidden_views, 0);

    // Opening a thread is a view of its note.
    let dave = ctx.member("dave").await;
    let _: GetNoteThreadResponse = ctx
        .note("GetNoteThread", Some(&dave), &GetNoteThreadRequest { note_id: other.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(view_count(&ctx, &other).await, 2);

    let too_many = RecordNoteViewsRequest { note_ids: vec![note.id.clone(); 101], ..Default::default() };
    assert_eq!(ctx.note_reason("RecordNoteViews", Some(&bob), &too_many).await, "TOO_MANY_NOTES");
    let malformed = RecordNoteViewsRequest { note_ids: vec!["nope".into()], ..Default::default() };
    assert_eq!(ctx.note_reason("RecordNoteViews", Some(&bob), &malformed).await, "INVALID_ID");
    let _: RecordNoteViewsResponse =
        ctx.note("RecordNoteViews", Some(&bob), &RecordNoteViewsRequest::default()).await.unwrap();
}

#[tokio::test]
async fn counters_stay_exact_under_concurrent_reactions() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    let note = ctx.post(&alice, "everyone at once").await;
    let mut fans = Vec::new();
    for n in 0..8 {
        fans.push(ctx.member(&format!("fan{n}")).await);
    }

    // Eight people like and renote at the same moment, each of them twice.
    let taps = fans.iter().flat_map(|fan| [fan, fan]).map(|fan| async {
        ctx.like(fan, &note).await;
        ctx.renote(fan, &note).await;
    });
    futures::future::join_all(taps).await;
    let counters = ctx.get_note(None, &note.id).await.unwrap().counters.as_option().cloned().unwrap();
    assert_eq!((counters.like_count, counters.renote_count), (8, 8));

    // Half take their like back while the other half reply.
    let (leaving, replying) = fans.split_at(4);
    let unlikes = leaving.iter().map(|fan| async { unlike(&ctx, fan, &note).await.like_count });
    let replies = replying.iter().map(|fan| async { ctx.reply(fan, &note, "me too").await.id });
    let (_, replies) =
        futures::future::join(futures::future::join_all(unlikes), futures::future::join_all(replies)).await;
    let counters = ctx.get_note(None, &note.id).await.unwrap().counters.as_option().cloned().unwrap();
    assert_eq!((counters.like_count, counters.reply_count), (4, 4));

    // The note and two of its replies are deleted while another reply comes in.
    let deletions = replies.iter().take(2).zip(replying).map(|(reply_id, fan)| {
        let request = DeleteNoteRequest { note_id: reply_id.clone(), ..Default::default() };
        let ctx = &ctx;
        async move { ctx.note::<_, DeleteNoteResponse>("DeleteNote", Some(fan), &request).await.map(|_| ()) }
    });
    let late_reply = async {
        ctx.note::<_, CreateNoteResponse>(
            "CreateNote",
            Some(&fans[0]),
            &CreateNoteRequest { text: "late".into(), reply_to_id: note.id.clone(), ..Default::default() },
        )
        .await
    };
    let (deleted, late) = futures::future::join(futures::future::join_all(deletions), late_reply).await;
    assert!(deleted.iter().all(Result::is_ok) && late.is_ok());
    assert_eq!(ctx.get_note(None, &note.id).await.unwrap().counters.reply_count, 3);
}
