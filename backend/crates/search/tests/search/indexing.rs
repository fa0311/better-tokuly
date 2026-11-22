//! The write path: `search.changed` events → registry → Meilisearch.
use tokuly_core::outbound::SEARCH_TOPIC;
use tokuly_proto::search::*;
use tokuly_search::{
    consumer::{CONSUMER_NAME, TOPICS},
    indexes::{index_settings, settings_match},
    kinds::KINDS,
    registry::Applied,
};
use uuid::Uuid;

use crate::common::*;

#[tokio::test]
async fn a_published_document_becomes_searchable_and_an_unindexed_one_disappears() {
    let ctx = start().await;
    let word = unique("pub");
    let video = doc("video", None, &format!("{word} の動画"));

    ctx.publish(&video).await;
    ctx.flush().await;
    assert_eq!(hit_ids(&ctx.find(None, &word).await.hits), vec![video.id.to_string()]);

    ctx.unpublish("video", video.id).await;
    ctx.flush().await;
    assert!(ctx.find(None, &word).await.hits.is_empty());
}

#[tokio::test]
async fn an_update_replaces_the_document() {
    let ctx = start().await;
    let (before, after) = (unique("old"), unique("new"));
    let mut note = doc("note", None, &before);
    ctx.index(std::slice::from_ref(&note)).await;

    note.title = after.clone();
    ctx.index(std::slice::from_ref(&note)).await;

    assert!(ctx.find(None, &before).await.hits.is_empty());
    let found = ctx.find(None, &after).await;
    assert_eq!(hit_ids(&found.hits), vec![note.id.to_string()]);
    assert_eq!(found.hits[0].title, after);
}

#[tokio::test]
async fn a_delete_never_loses_to_an_older_upsert() {
    let ctx = start().await;
    let word = unique("order");
    let video = doc("video", None, &word);
    // The upsert happened first, the delete second...
    let upsert = event_with_id(Uuid::now_v7(), SEARCH_TOPIC, upsert_of(&video));
    let delete = event_with_id(Uuid::now_v7(), SEARCH_TOPIC, removal_of("video", video.id));
    assert!(upsert.id < delete.id);

    // ...but they arrive the other way round.
    ctx.deliver(delete).await;
    ctx.deliver(upsert).await;
    ctx.flush().await;

    assert!(ctx.find(None, &word).await.hits.is_empty(), "the older upsert must not bring the document back");
}

#[tokio::test]
async fn an_older_upsert_never_overwrites_a_newer_one() {
    let ctx = start().await;
    let (stale, current) = (unique("stale"), unique("current"));
    let mut video = doc("video", None, &stale);
    let first = event_with_id(Uuid::now_v7(), SEARCH_TOPIC, upsert_of(&video));
    video.title = current.clone();
    let second = event_with_id(Uuid::now_v7(), SEARCH_TOPIC, upsert_of(&video));

    ctx.deliver(second).await;
    ctx.deliver(first).await;
    ctx.flush().await;

    assert_eq!(hit_ids(&ctx.find(None, &current).await.hits), vec![video.id.to_string()]);
    assert!(ctx.find(None, &stale).await.hits.is_empty());
}

#[tokio::test]
async fn a_newer_upsert_after_a_delete_brings_the_document_back() {
    let ctx = start().await;
    let word = unique("back");
    let video = doc("video", None, &word);
    ctx.index(std::slice::from_ref(&video)).await;
    ctx.unpublish("video", video.id).await;
    ctx.flush().await;
    assert!(ctx.find(None, &word).await.hits.is_empty());

    ctx.index(std::slice::from_ref(&video)).await;
    assert_eq!(hit_ids(&ctx.find(None, &word).await.hits), vec![video.id.to_string()]);
}

#[tokio::test]
async fn a_redelivered_event_changes_nothing() {
    let ctx = start().await;
    let video = doc("video", None, &unique("again"));
    let version = Uuid::now_v7();

    assert_eq!(ctx.search.apply_change(version, upsert_of(&video)).await.unwrap(), Applied::Stored);
    assert_eq!(ctx.search.apply_change(version, upsert_of(&video)).await.unwrap(), Applied::Stale);
    // Also through the consumer, and also after the document was removed.
    let delete = event(SEARCH_TOPIC, removal_of("video", video.id));
    ctx.deliver(delete.clone()).await;
    ctx.deliver(delete.clone()).await;
    assert_eq!(ctx.search.apply_change(delete.id, removal_of("video", video.id)).await.unwrap(), Applied::Stale);
    assert_eq!(ctx.search.apply_change(version, upsert_of(&video)).await.unwrap(), Applied::Stale);
}

#[tokio::test]
async fn republishing_an_unchanged_document_is_not_written_to_the_engine_again() {
    let ctx = start().await;
    let video = doc("video", None, &unique("same"));
    ctx.index(std::slice::from_ref(&video)).await;
    let generation = |id: Uuid| {
        let db = ctx.app.core.db.clone();
        async move {
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT generation, synced_generation FROM search_documents WHERE kind = 'video' AND doc_id = $1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let (before, synced) = generation(video.id).await;
    assert_eq!(before, synced, "the indexer marked the row as written");

    // A full republish by the owning domain: same content, newer event.
    ctx.publish(&video).await;
    assert_eq!(generation(video.id).await, (before, synced), "nothing to write");

    // A real change is pending again.
    let mut changed = video.clone();
    changed.score = 5.0;
    ctx.publish(&changed).await;
    assert_eq!(generation(video.id).await.0, before + 1);
    ctx.flush().await;
}

#[tokio::test]
async fn a_document_that_stops_being_public_is_removed() {
    let ctx = start().await;
    let word = unique("private");
    let mut note = doc("note", None, &word);
    ctx.index(std::slice::from_ref(&note)).await;
    assert_eq!(ctx.find(None, &word).await.hits.len(), 1);

    // `outbound::index` with `public = false` emits a removal; a producer
    // that sends the document anyway is treated the same.
    note.public = false;
    let mut leaky = upsert_of(&note);
    leaky.doc = Some(note.clone());
    ctx.deliver(event(SEARCH_TOPIC, leaky)).await;
    ctx.flush().await;
    assert!(ctx.find(None, &word).await.hits.is_empty());
}

#[tokio::test]
async fn malformed_changes_are_dropped_instead_of_retried_forever() {
    let ctx = start().await;
    let word = unique("bad");

    // An unknown kind.
    let alien = doc("spaceship", None, &word);
    assert_eq!(ctx.search.apply_change(Uuid::now_v7(), upsert_of(&alien)).await.unwrap(), Applied::Rejected);
    // A document that is not the one the change is about.
    let video = doc("video", None, &word);
    let mut mismatched = upsert_of(&video);
    mismatched.id = Uuid::now_v7();
    assert_eq!(ctx.search.apply_change(Uuid::now_v7(), mismatched).await.unwrap(), Applied::Rejected);
    // Payloads that are not a change at all are acknowledged, not failed.
    ctx.search.handle_event(event(SEARCH_TOPIC, serde_json::json!({ "what": "ever" }))).await.unwrap();
    ctx.search.handle_event(event("identity.user.updated", serde_json::json!("nope"))).await.unwrap();
    // Topics the consumer does not know are ignored.
    ctx.search.handle_event(event("chat.message.sent", serde_json::json!({}))).await.unwrap();

    ctx.flush().await;
    assert!(ctx.find(None, &word).await.hits.is_empty());
}

#[tokio::test]
async fn the_indexer_writes_many_kinds_in_one_pass() {
    let ctx = start().await;
    let owner = ctx.app.user("batcher").await;
    let word = unique("batch");
    let mut docs = Vec::new();
    for kind in KINDS.iter().filter(|kind| kind.name != "user") {
        for n in 0..3 {
            docs.push(doc(kind.name, Some(owner.id), &format!("{word} {} {n}", kind.name)));
        }
    }
    ctx.index(&docs).await;

    let found = ctx
        .search(None, &SearchRequest { query: word.clone(), page: Some(page(50)).into(), ..Default::default() })
        .await;
    // Three of every kind but `user`; they fit on one page.
    assert_eq!(found.hits.len(), docs.len());
    assert_eq!(found.total, docs.len() as i64);
    for kind in KINDS.iter().filter(|kind| kind.name != "user") {
        assert_eq!(count_of(&found, kind.proto), 3, "{}", kind.name);
    }

    // Removing in bulk works the same way.
    for doc in &docs {
        ctx.unpublish(&doc.kind, doc.id).await;
    }
    ctx.flush().await;
    assert_eq!(ctx.find(None, &word).await.total, 0);
}

fn page(limit: i32) -> tokuly_proto::common::PageRequest {
    tokuly_proto::common::PageRequest { limit, ..Default::default() }
}

#[tokio::test]
async fn a_pass_is_skipped_while_another_process_is_indexing() {
    let ctx = start().await;
    // Another worker process in the middle of a pass holds the indexer lock.
    let mut other_worker = ctx.app.core.db.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("search:indexer:{}", ctx.search.cfg.index_prefix))
        .execute(&mut *other_worker)
        .await
        .unwrap();

    assert_eq!(ctx.search.flush_once().await.unwrap(), None);

    // Once it is done, passes run again (`flush` queues behind other tests').
    other_worker.rollback().await.unwrap();
    ctx.search.flush().await.unwrap();
}

#[tokio::test]
async fn an_engine_outage_fails_the_pass_and_loses_nothing() {
    let ctx = start().await;
    // A worker whose Meilisearch is unreachable (nothing listens on port 9),
    // with its own index prefix and therefore its own indexer lock.
    let mut cfg = ctx.app.core.cfg.clone();
    cfg.meili_url = "http://127.0.0.1:9".to_owned();
    let offline_core = tokuly_core::Core::connect(cfg).await.unwrap();
    let offline = tokuly_search::Search::new(
        offline_core,
        ctx.app.identity.clone(),
        tokuly_search::SearchConfig { index_prefix: format!("{}off", *PREFIX), ..ctx.search.cfg.clone() },
    )
    .unwrap();

    let setup = offline.ensure_indexes().await.unwrap_err();
    assert_eq!((setup.code, setup.reason.as_ref()), (connectrpc::ErrorCode::Unavailable, "SEARCH_UNAVAILABLE"));

    // A change arrives during the outage. The healthy indexers are kept
    // away, so the pass that fails is certain to include it.
    let word = unique("outage");
    let video = doc("video", None, &word);
    let mut healthy_indexers = ctx.app.core.db.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("search:indexer:{}", ctx.search.cfg.index_prefix))
        .execute(&mut *healthy_indexers)
        .await
        .unwrap();
    offline.handle_event(event(SEARCH_TOPIC, upsert_of(&video))).await.expect("the registry does not need the engine");
    let failed = offline.flush_once().await.unwrap_err();
    assert_eq!(failed.reason, "SEARCH_UNAVAILABLE");
    let (generation, synced): (i64, i64) = sqlx::query_as(
        "SELECT generation, synced_generation FROM search_documents WHERE kind = 'video' AND doc_id = $1",
    )
    .bind(video.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert!(synced < generation, "a failed pass marks nothing as written");

    // When an engine is reachable again the change is written.
    healthy_indexers.rollback().await.unwrap();
    ctx.flush().await;
    assert_eq!(hit_ids(&ctx.find(None, &word).await.hits), vec![video.id.to_string()]);
}

#[tokio::test]
async fn index_setup_is_idempotent_and_applies_the_settings() {
    let ctx = start().await;
    // The harness already ran it; again and again changes nothing.
    ctx.search.ensure_indexes().await.unwrap();
    ctx.search.setup().await.unwrap();

    let desired = index_settings(Default::default());
    for kind in KINDS {
        let uid = ctx.search.index_uid(kind);
        assert_eq!(uid, format!("{}_{}", *PREFIX, kind.name));
        let index = ctx.app.core.search.get_index(&uid).await.unwrap_or_else(|e| panic!("{uid} exists: {e}"));
        assert_eq!(index.primary_key.as_deref(), Some("id"));
        let settings = index.get_settings().await.unwrap();
        // Synonyms belong to the admin tests, which may be changing them.
        let comparable = meilisearch_sdk::settings::Settings { synonyms: desired.synonyms.clone(), ..settings.clone() };
        assert!(settings_match(&comparable, &desired), "{uid} has the managed settings");
        let locales = &settings.localized_attributes.as_ref().expect("localized attributes")[0];
        assert_eq!(locales.locales, vec!["jpn", "eng"]);
        assert!(locales.attribute_patterns.contains(&"title".to_owned()));
        let typo = settings.typo_tolerance.expect("typo tolerance");
        assert_eq!(typo.disable_on_attributes, Some(vec!["handle".to_owned()]));
    }
}

#[tokio::test]
async fn the_retention_sweep_removes_only_tombstones_past_their_keep_by_date() {
    let ctx = start().await;
    let (old, recent, pending) = (
        doc("clip", None, &unique("oldtomb")),
        doc("clip", None, &unique("newtomb")),
        doc("clip", None, &unique("pendtomb")),
    );
    ctx.index(&[old.clone(), recent.clone(), pending.clone()]).await;
    ctx.unpublish("clip", old.id).await;
    ctx.unpublish("clip", recent.id).await;
    ctx.flush().await;
    // Removed, but the engine has not been told yet: no pass may run (not
    // even another test's) until the sweep was observed.
    let mut indexer_lock = ctx.app.core.db.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("search:indexer:{}", ctx.search.cfg.index_prefix))
        .execute(&mut *indexer_lock)
        .await
        .unwrap();
    ctx.unpublish("clip", pending.id).await;
    sqlx::query("UPDATE search_documents SET updated_at = now() - interval '9 days' WHERE doc_id = ANY($1)")
        .bind(vec![old.id, pending.id])
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    let report = ctx.search.run_retention().await.unwrap();
    assert!(report.tombstones >= 1);

    let left: Vec<Uuid> = sqlx::query_scalar("SELECT doc_id FROM search_documents WHERE doc_id = ANY($1)")
        .bind(vec![old.id, recent.id, pending.id])
        .fetch_all(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(!left.contains(&old.id), "an old, written tombstone is swept");
    assert!(left.contains(&recent.id), "a recent tombstone still guards against late upserts");
    assert!(left.contains(&pending.id), "a removal the engine has not seen is never swept");
    indexer_lock.rollback().await.unwrap();
    ctx.flush().await;
}

#[tokio::test]
async fn a_takedown_removes_the_subjects_document() {
    let ctx = start().await;
    let word = unique("takedown");
    let (video, note) = (doc("video", None, &word), doc("note", None, &word));
    ctx.index(&[video.clone(), note.clone()]).await;

    let removed = serde_json::json!({
        "subject_type": tokuly_proto::common::SubjectType::SUBJECT_TYPE_VIDEO as i32,
        "subject_id": video.id,
        "reason": "spam",
        "moderator_id": Uuid::now_v7(),
    });
    ctx.deliver(event("admin.content.removed", removed)).await;
    // A subject type that is not indexed is none of our business.
    let comment = serde_json::json!({
        "subject_type": tokuly_proto::common::SubjectType::SUBJECT_TYPE_COMMENT as i32,
        "subject_id": note.id,
        "reason": "spam",
        "moderator_id": Uuid::now_v7(),
    });
    ctx.deliver(event("admin.content.removed", comment)).await;
    ctx.flush().await;

    assert_eq!(hit_ids(&ctx.find(None, &word).await.hits), vec![note.id.to_string()]);
}

#[test]
fn the_consumer_listens_to_everything_that_changes_search() {
    assert_eq!(CONSUMER_NAME, "search");
    for topic in [
        "search.changed",
        "identity.user.created",
        "identity.user.updated",
        "identity.user.privacy_changed",
        "identity.user.banned",
        "identity.user.unbanned",
        "identity.user.deleted",
        "admin.content.removed",
    ] {
        assert!(TOPICS.contains(&topic), "{topic}");
    }
}
