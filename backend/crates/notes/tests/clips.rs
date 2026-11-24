//! Clips: collections of notes, what each reader sees of them, and the
//! public clip count of notes.
mod common;

use common::*;
use futures::future::join_all;
use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_proto::{
    common::Visibility,
    notes::{notes_event::Event as Push, *},
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

fn event(topic: &str, data: serde_json::Value) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: None, occurred_at: chrono::Utc::now(), data }
}

trait Clips {
    async fn clip<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError>;
    async fn clip_reason<Req: Serialize>(&self, method: &str, user: Option<&TestUser>, request: &Req) -> String;
    async fn create_clip(&self, owner: &TestUser, name: &str, public: bool) -> Clip;
    async fn add(&self, owner: &TestUser, clip: &Clip, note: &Note) -> Result<Clip, RpcError>;
    async fn remove(&self, owner: &TestUser, clip: &Clip, note: &Note) -> Clip;
    async fn get_clip(&self, viewer: Option<&TestUser>, clip: &Clip) -> Result<Clip, RpcError>;
    async fn clip_notes(&self, viewer: Option<&TestUser>, clip: &Clip) -> Vec<Note>;
    async fn clip_count(&self, note: &Note) -> i64;
}

impl Clips for Ctx {
    async fn clip<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.notes.v1.ClipService/{method}"), user, request).await
    }

    async fn clip_reason<Req: Serialize>(&self, method: &str, user: Option<&TestUser>, request: &Req) -> String {
        match self.clip::<_, serde_json::Value>(method, user, request).await {
            Ok(response) => panic!("ClipService/{method} should fail, got {response}"),
            Err(error) => error.reason,
        }
    }

    async fn create_clip(&self, owner: &TestUser, name: &str, public: bool) -> Clip {
        let request = CreateClipRequest { name: name.to_owned(), public, ..Default::default() };
        let created: CreateClipResponse = self.clip("CreateClip", Some(owner), &request).await.unwrap();
        created.clip.as_option().cloned().unwrap()
    }

    async fn add(&self, owner: &TestUser, clip: &Clip, note: &Note) -> Result<Clip, RpcError> {
        let request = AddNoteToClipRequest { clip_id: clip.id.clone(), note_id: note.id.clone(), ..Default::default() };
        let added: AddNoteToClipResponse = self.clip("AddNoteToClip", Some(owner), &request).await?;
        Ok(added.clip.as_option().cloned().unwrap())
    }

    async fn remove(&self, owner: &TestUser, clip: &Clip, note: &Note) -> Clip {
        let request =
            RemoveNoteFromClipRequest { clip_id: clip.id.clone(), note_id: note.id.clone(), ..Default::default() };
        let removed: RemoveNoteFromClipResponse = self.clip("RemoveNoteFromClip", Some(owner), &request).await.unwrap();
        removed.clip.as_option().cloned().unwrap()
    }

    async fn get_clip(&self, viewer: Option<&TestUser>, clip: &Clip) -> Result<Clip, RpcError> {
        let got: GetClipResponse =
            self.clip("GetClip", viewer, &GetClipRequest { clip_id: clip.id.clone(), ..Default::default() }).await?;
        Ok(got.clip.as_option().cloned().unwrap())
    }

    async fn clip_notes(&self, viewer: Option<&TestUser>, clip: &Clip) -> Vec<Note> {
        let request = GetClipNotesRequest { clip_id: clip.id.clone(), page: page(50, ""), ..Default::default() };
        let got: GetClipNotesResponse = self.clip("GetClipNotes", viewer, &request).await.unwrap();
        got.notes
    }

    async fn clip_count(&self, note: &Note) -> i64 {
        sqlx::query_scalar("SELECT clip_count FROM notes WHERE id = $1")
            .bind(id(&note.id))
            .fetch_one(&self.app.core.db)
            .await
            .unwrap()
    }
}

fn list_of(user: &TestUser, note: Option<&Note>) -> ListClipsRequest {
    ListClipsRequest {
        user_id: user.id.to_string(),
        note_id: note.map(|note| note.id.clone()).unwrap_or_default(),
        page: page(50, ""),
        ..Default::default()
    }
}

fn names(clips: &[Clip]) -> Vec<&str> {
    clips.iter().map(|clip| clip.name.as_str()).collect()
}

#[tokio::test]
async fn clips_are_created_edited_and_deleted_by_their_owner() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let request = CreateClipRequest {
        name: "  あとで  読む ".into(),
        description: "気になったノート".into(),
        public: false,
        ..Default::default()
    };
    let created: CreateClipResponse = ctx.clip("CreateClip", Some(&alice), &request).await.unwrap();
    let clip = created.clip.as_option().cloned().unwrap();
    assert_eq!(
        (clip.name.as_str(), clip.description.as_str(), clip.public),
        ("あとで 読む", "気になったノート", false)
    );
    assert_eq!(clip.owner.id, alice.id.to_string());
    assert_eq!(clip.note_count, 0);

    for (request, reason) in [
        (CreateClipRequest { name: " ".into(), ..Default::default() }, "INVALID_NAME"),
        (CreateClipRequest { name: "x".repeat(51), ..Default::default() }, "INVALID_NAME"),
        (
            CreateClipRequest { name: "ok".into(), description: "x".repeat(501), ..Default::default() },
            "DESCRIPTION_TOO_LONG",
        ),
    ] {
        assert_eq!(ctx.clip_reason("CreateClip", Some(&alice), &request).await, reason);
    }
    let anonymous = ctx
        .clip::<_, CreateClipResponse>(
            "CreateClip",
            None,
            &CreateClipRequest { name: "x".into(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(anonymous.code, "unauthenticated");

    let update = UpdateClipRequest {
        clip_id: clip.id.clone(),
        name: "best".into(),
        description: String::new(),
        public: true,
        ..Default::default()
    };
    let updated: UpdateClipResponse = ctx.clip("UpdateClip", Some(&alice), &update).await.unwrap();
    assert!(updated.clip.public && updated.clip.name == "best");
    assert_eq!(ctx.clip_reason("UpdateClip", Some(&bob), &update).await, "CLIP_NOT_FOUND");
    let invalid = UpdateClipRequest { name: String::new(), ..update.clone() };
    assert_eq!(ctx.clip_reason("UpdateClip", Some(&alice), &invalid).await, "INVALID_NAME");
    let invalid = UpdateClipRequest { description: "x".repeat(501), ..update.clone() };
    assert_eq!(ctx.clip_reason("UpdateClip", Some(&alice), &invalid).await, "DESCRIPTION_TOO_LONG");

    let delete = DeleteClipRequest { clip_id: clip.id.clone(), ..Default::default() };
    assert_eq!(ctx.clip_reason("DeleteClip", Some(&bob), &delete).await, "CLIP_NOT_FOUND");
    let _: DeleteClipResponse = ctx.clip("DeleteClip", Some(&alice), &delete).await.unwrap();
    assert_eq!(ctx.get_clip(Some(&alice), &clip).await.unwrap_err().reason, "CLIP_NOT_FOUND");
    assert_eq!(ctx.clip_reason("DeleteClip", Some(&alice), &delete).await, "CLIP_NOT_FOUND");
}

#[tokio::test]
async fn one_person_has_at_most_a_hundred_clips_even_when_creating_at_once() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    sqlx::query("INSERT INTO note_clips (owner_id, name) SELECT $1, 'clip ' || g FROM generate_series(1, 99) g")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let results = join_all((0..4).map(|i| {
        let request = CreateClipRequest { name: format!("new {i}"), ..Default::default() };
        let (ctx, alice) = (&ctx, &alice);
        async move { ctx.clip::<_, CreateClipResponse>("CreateClip", Some(alice), &request).await }
    }))
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(results.into_iter().filter_map(Result::err).all(|e| e.reason == "TOO_MANY_CLIPS"));
}

#[tokio::test]
async fn notes_go_into_clips_once_and_only_notes_the_clipper_may_see() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    let clip = ctx.create_clip(&alice, "picks", false).await;
    let public = ctx.post(&bob, "public").await;
    let friends_only = ctx.post_visible_to(&bob, "friends only", Visibility::VISIBILITY_FRIENDS).await;

    let added = ctx.add(&alice, &clip, &public).await.unwrap();
    assert!(added.contains_note);
    assert_eq!(added.note_count, 1);
    assert_eq!(ctx.add(&alice, &clip, &public).await.unwrap().note_count, 1, "adding twice changes nothing");
    assert_eq!(ctx.add(&alice, &clip, &friends_only).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    let missing = Note { id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.add(&alice, &clip, &missing).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    // Someone else's clip is not there for them, even public.
    assert_eq!(ctx.add(&carol, &clip, &public).await.unwrap_err().reason, "CLIP_NOT_FOUND");
    let carols = ctx.create_clip(&carol, "public one", true).await;
    assert_eq!(ctx.add(&alice, &carols, &public).await.unwrap_err().reason, "CLIP_NOT_FOUND");

    let removed = ctx.remove(&alice, &clip, &public).await;
    assert!(!removed.contains_note && removed.note_count == 0);
    ctx.remove(&alice, &clip, &public).await; // already gone: fine
    let request =
        RemoveNoteFromClipRequest { clip_id: clip.id.clone(), note_id: public.id.clone(), ..Default::default() };
    assert_eq!(ctx.clip_reason("RemoveNoteFromClip", Some(&bob), &request).await, "CLIP_NOT_FOUND");
}

#[tokio::test]
async fn a_clip_holds_at_most_a_thousand_notes_even_when_adding_at_once() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let clip = ctx.create_clip(&alice, "full", true).await;
    sqlx::query(
        "WITH made AS (INSERT INTO notes (author_id, visibility, listed, text) SELECT $1, 1, true, 'filler' FROM generate_series(1, 999) RETURNING id)
         INSERT INTO note_clip_items (clip_id, note_id) SELECT $2, id FROM made",
    )
    .bind(bob.id)
    .bind(id(&clip.id))
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let mut notes = Vec::new();
    for i in 0..3 {
        notes.push(ctx.post(&bob, &format!("last {i}")).await);
    }
    let results = join_all(notes.iter().map(|note| ctx.add(&alice, &clip, note))).await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(results.iter().filter_map(|r| r.as_ref().err()).all(|e| e.reason == "TOO_MANY_CLIPPED_NOTES"));
    let counted: i64 = notes.iter().zip(&results).filter(|(_, r)| r.is_ok()).count() as i64;
    let mut total = 0;
    for note in &notes {
        total += ctx.clip_count(note).await;
    }
    assert_eq!(total, counted, "only the note that got in is counted");
}

#[tokio::test]
async fn the_clip_count_follows_public_clips_exactly() {
    let ctx = start().await;
    let (author, alice) = (ctx.member("author").await, ctx.member("alice").await);
    let note = ctx.post(&author, "clip me").await;
    let mut watch = ctx.app.listen_topic(&tokuly_notes::model::note_topic(id(&note.id))).await;

    let public = ctx.create_clip(&alice, "public", true).await;
    let private = ctx.create_clip(&alice, "private", false).await;
    ctx.add(&alice, &public, &note).await.unwrap();
    ctx.add(&alice, &private, &note).await.unwrap();
    assert_eq!(ctx.clip_count(&note).await, 1, "a private clip does not count");
    let pushed = next_push(&mut watch).await.expect("fresh counters");
    assert!(matches!(pushed, Push::CountersChanged(counters) if counters.clip_count == 1));
    assert_eq!(ctx.get_note(None, &note.id).await.unwrap().counters.clip_count, 1);

    // Turning clips public and private moves it.
    let update = |clip: &Clip, public: bool| UpdateClipRequest {
        clip_id: clip.id.clone(),
        name: clip.name.clone(),
        public,
        ..Default::default()
    };
    let _: UpdateClipResponse = ctx.clip("UpdateClip", Some(&alice), &update(&private, true)).await.unwrap();
    assert_eq!(ctx.clip_count(&note).await, 2);
    let _: UpdateClipResponse = ctx.clip("UpdateClip", Some(&alice), &update(&private, true)).await.unwrap();
    assert_eq!(ctx.clip_count(&note).await, 2, "no change, no move");
    let _: UpdateClipResponse = ctx.clip("UpdateClip", Some(&alice), &update(&public, false)).await.unwrap();
    assert_eq!(ctx.clip_count(&note).await, 1);
    let delete = DeleteClipRequest { clip_id: private.id.clone(), ..Default::default() };
    let _: DeleteClipResponse = ctx.clip("DeleteClip", Some(&alice), &delete).await.unwrap();
    assert_eq!(ctx.clip_count(&note).await, 0);

    // Many public clips adding and removing at once.
    let mut clippers = Vec::new();
    for i in 0..5 {
        let user = ctx.member(&format!("clipper{i}")).await;
        let clip = ctx.create_clip(&user, "mine", true).await;
        clippers.push((user, clip));
    }
    join_all(clippers.iter().chain(clippers.iter()).map(|(user, clip)| ctx.add(user, clip, &note))).await;
    assert_eq!(ctx.clip_count(&note).await, 5);
    join_all(clippers.iter().take(2).chain(clippers.iter().take(2)).map(|(user, clip)| ctx.remove(user, clip, &note)))
        .await;
    assert_eq!(ctx.clip_count(&note).await, 3);
    // Adds and removes of one clip racing each other end consistent.
    let (user, clip) = &clippers[4];
    let mut races = Vec::new();
    let (shared, target) = (&ctx, &note);
    for i in 0..6 {
        races.push(async move {
            if i % 2 == 0 {
                shared.add(user, clip, target).await.map(|_| ())
            } else {
                shared.remove(user, clip, target).await;
                Ok(())
            }
        });
    }
    join_all(races).await;
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM note_clip_items i JOIN note_clips c ON c.id = i.clip_id WHERE i.note_id = $1 AND c.public",
    )
    .bind(id(&note.id))
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(ctx.clip_count(&note).await, held);
}

#[tokio::test]
async fn readers_see_only_the_clipped_notes_they_may_see_and_only_counted_as_such() {
    let ctx = start().await;
    let (owner, author, follower, stranger) = (
        ctx.member("owner").await,
        ctx.member("author").await,
        ctx.member("follower").await,
        ctx.member("stranger").await,
    );
    ctx.follow(&owner, &author).await;
    ctx.follow(&follower, &author).await;
    let public_note = ctx.post(&author, "for everyone").await;
    let followers_note = ctx.post_visible_to(&author, "for followers", Visibility::VISIBILITY_FOLLOWERS).await;
    let clip = ctx.create_clip(&owner, "shared", true).await;
    ctx.add(&owner, &clip, &public_note).await.unwrap();
    ctx.add(&owner, &clip, &followers_note).await.unwrap();
    let hidden = ctx.create_clip(&owner, "mine only", false).await;
    ctx.add(&owner, &hidden, &public_note).await.unwrap();

    assert_eq!(ctx.get_clip(Some(&owner), &clip).await.unwrap().note_count, 2);
    assert_eq!(ctx.get_clip(Some(&follower), &clip).await.unwrap().note_count, 2);
    assert_eq!(ctx.get_clip(Some(&stranger), &clip).await.unwrap().note_count, 1);
    assert_eq!(ctx.get_clip(None, &clip).await.unwrap().note_count, 1);
    assert_eq!(note_texts(&ctx.clip_notes(Some(&follower), &clip).await), ["for followers", "for everyone"]);
    assert_eq!(note_texts(&ctx.clip_notes(Some(&stranger), &clip).await), ["for everyone"]);
    assert_eq!(note_texts(&ctx.clip_notes(None, &clip).await), ["for everyone"]);
    // A private clip is not there for anyone else.
    assert_eq!(ctx.get_clip(Some(&stranger), &hidden).await.unwrap_err().reason, "CLIP_NOT_FOUND");
    let request = GetClipNotesRequest { clip_id: hidden.id.clone(), ..Default::default() };
    assert_eq!(ctx.clip_reason("GetClipNotes", Some(&stranger), &request).await, "CLIP_NOT_FOUND");

    // Their list: all for the owner, public for others; "contains" only for a note one may see.
    let mine: ListClipsResponse = ctx.clip("ListClips", Some(&owner), &list_of(&owner, None)).await.unwrap();
    assert_eq!(names(&mine.clips), ["mine only", "shared"]);
    let empty_means_me = ListClipsRequest::default();
    let mine_again: ListClipsResponse = ctx.clip("ListClips", Some(&owner), &empty_means_me).await.unwrap();
    assert_eq!(mine_again.clips.len(), 2);
    let theirs: ListClipsResponse =
        ctx.clip("ListClips", Some(&stranger), &list_of(&owner, Some(&followers_note))).await.unwrap();
    assert_eq!(names(&theirs.clips), ["shared"]);
    assert!(!theirs.clips[0].contains_note, "a hidden note is not confirmed through a clip");
    assert_eq!(theirs.clips[0].note_count, 1);
    let theirs: ListClipsResponse =
        ctx.clip("ListClips", Some(&follower), &list_of(&owner, Some(&followers_note))).await.unwrap();
    assert!(theirs.clips[0].contains_note);
    let anonymous: ListClipsResponse = ctx.clip("ListClips", None, &list_of(&owner, None)).await.unwrap();
    assert_eq!(names(&anonymous.clips), ["shared"]);
    let nobody = ListClipsRequest { user_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.clip_reason("ListClips", None, &nobody).await, "USER_NOT_FOUND");

    // A block hides the owner's clips both ways.
    ctx.block(&owner, &stranger).await;
    assert_eq!(ctx.get_clip(Some(&stranger), &clip).await.unwrap_err().reason, "CLIP_NOT_FOUND");
    assert_eq!(ctx.clip_reason("ListClips", Some(&stranger), &list_of(&owner, None)).await, "USER_NOT_FOUND");
}

#[tokio::test]
async fn a_note_lists_the_public_clips_holding_it_and_the_readers_own() {
    let ctx = start().await;
    let (author, alice, bob, reader) =
        (ctx.member("author").await, ctx.member("alice").await, ctx.member("bob").await, ctx.member("reader").await);
    let note = ctx.post(&author, "popular").await;
    let alices = ctx.create_clip(&alice, "alice public", true).await;
    let bobs_private = ctx.create_clip(&bob, "bob private", false).await;
    let readers_private = ctx.create_clip(&reader, "reader private", false).await;
    for (user, clip) in [(&alice, &alices), (&bob, &bobs_private), (&reader, &readers_private)] {
        ctx.add(user, clip, &note).await.unwrap();
    }
    let request = ListNoteClipsRequest { note_id: note.id.clone(), page: page(50, ""), ..Default::default() };
    let seen: ListNoteClipsResponse = ctx.clip("ListNoteClips", Some(&reader), &request).await.unwrap();
    assert_eq!(names(&seen.clips), ["reader private", "alice public"]);
    assert!(seen.clips.iter().all(|clip| clip.contains_note));
    let anonymous: ListNoteClipsResponse = ctx.clip("ListNoteClips", None, &request).await.unwrap();
    assert_eq!(names(&anonymous.clips), ["alice public"]);

    let private = ctx.post_visible_to(&author, "private", Visibility::VISIBILITY_PRIVATE).await;
    let request = ListNoteClipsRequest { note_id: private.id.clone(), ..Default::default() };
    assert_eq!(ctx.clip_reason("ListNoteClips", Some(&reader), &request).await, "NOTE_NOT_FOUND");
}

#[tokio::test]
async fn deleting_an_account_takes_its_clips_and_puts_the_counts_right() {
    let ctx = start().await;
    let (author, alice) = (ctx.member("author").await, ctx.member("alice").await);
    let note = ctx.post(&author, "clipped").await;
    let clip = ctx.create_clip(&alice, "alice's", true).await;
    ctx.add(&alice, &clip, &note).await.unwrap();
    let authors = ctx.create_clip(&author, "author's", true).await;
    let alices_note = ctx.post(&alice, "by alice").await;
    ctx.add(&author, &authors, &alices_note).await.unwrap();
    assert_eq!(ctx.clip_count(&note).await, 1);

    let deleted = event("identity.user.deleted", serde_json::json!({ "user_id": alice.id }));
    ctx.notes.handle_event(deleted.clone()).await.unwrap();
    ctx.notes.handle_event(deleted).await.unwrap();
    assert_eq!(ctx.clip_count(&note).await, 0);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM note_clips WHERE owner_id = $1")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
    // Her note left the author's clip with it.
    assert_eq!(ctx.get_clip(Some(&author), &authors).await.unwrap().note_count, 0);
}

#[tokio::test]
async fn clips_are_in_the_data_export() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    let note = ctx.post(&alice, "keep").await;
    let clip = ctx.create_clip(&alice, "exported", true).await;
    ctx.add(&alice, &clip, &note).await.unwrap();
    let exporter = ctx.app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "notes").unwrap();
    let export = exporter.export(alice.id).await.unwrap();
    assert_eq!(export["clips"][0]["name"], "exported");
    assert_eq!(export["clips"][0]["note_ids"], serde_json::json!([note.id]));
}

#[tokio::test]
async fn locking_clip_calls_from_one_account_do_not_starve_the_pool() {
    let ctx = start().await;
    let (alice, author) = (ctx.member("alice").await, ctx.member("author").await);
    let clip = ctx.create_clip(&alice, "busy", true).await;
    let mut notes = Vec::new();
    for i in 0..4 {
        notes.push(ctx.post(&author, &format!("n{i}")).await);
    }
    let started = std::time::Instant::now();
    let adds = notes.iter().map(|note| ctx.add(&alice, &clip, note));
    let creates = (0..4).map(|i| {
        let request = CreateClipRequest { name: format!("c{i}"), public: true, ..Default::default() };
        let (ctx, alice) = (&ctx, &alice);
        async move { ctx.clip::<_, CreateClipResponse>("CreateClip", Some(alice), &request).await.map(|_| ()) }
    });
    let toggles = (0..3).map(|i| {
        let request = UpdateClipRequest {
            clip_id: clip.id.clone(),
            name: "busy".into(),
            public: i % 2 == 0,
            ..Default::default()
        };
        let (ctx, alice) = (&ctx, &alice);
        async move { ctx.clip::<_, UpdateClipResponse>("UpdateClip", Some(alice), &request).await.map(|_| ()) }
    });
    let (adds, creates, toggles) = tokio::join!(join_all(adds), join_all(creates), join_all(toggles));
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "calls waited on the pool");
    assert!(adds.iter().all(Result::is_ok) && creates.iter().all(Result::is_ok) && toggles.iter().all(Result::is_ok));
    let public: bool = sqlx::query_scalar("SELECT public FROM note_clips WHERE id = $1")
        .bind(id(&clip.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    for note in &notes {
        assert_eq!(ctx.clip_count(note).await, i64::from(public));
    }
}
