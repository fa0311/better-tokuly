//! The whole path with the real workers: another domain's transaction →
//! outbox → event stream → search consumer → registry → indexer →
//! Meilisearch, and the jobs an administrator's reindex starts.
//!
//! Its own test binary: its own NATS namespace, database and index prefix,
//! so the workers started here only ever see this test's work. Everything
//! here is asynchronous across processes in production, so the test polls
//! the public API with a deadline; the write to the engine itself is still
//! awaited by the indexer through the task API.
use std::time::{Duration, Instant};

use tokuly_core::outbound;
use tokuly_proto::search::*;

#[path = "search/common.rs"]
mod common;
use common::*;

/// Polls until `check` holds, failing the test after 60 seconds.
async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn documents_published_by_other_domains_become_searchable_through_the_workers() {
    let ctx = start_with(|cfg| {
        cfg.index_interval_ms = 50;
        cfg.trending_refresh_seconds = 1;
    })
    .await;
    let core = &ctx.app.core;
    let admin = ctx.app.user_with_roles("admin", &["admin"]).await;
    let creator = ctx.app.user("creator").await;

    // What a worker process runs.
    ctx.search.spawn_workers();
    tokio::spawn(tokuly_core::events::run_outbox_relay(core.db.clone(), core.bus.clone(), core.shutdown.clone()));

    // Another domain publishes a document inside its own transaction.
    let word = unique("pipeline");
    let video = doc("video", Some(creator.id), &format!("{word} の動画"));
    let mut tx = core.db.begin().await.unwrap();
    outbound::index(&mut *tx, video.clone()).await.unwrap();
    tx.commit().await.unwrap();
    eventually("the published video to be searchable", async || {
        hit_ids(&ctx.find(None, &word).await.hits) == vec![video.id.to_string()]
    })
    .await;

    // It changes, then it is unpublished.
    let renamed = unique("renamed");
    let mut tx = core.db.begin().await.unwrap();
    outbound::index(&mut *tx, tokuly_core::outbound::SearchDoc { title: renamed.clone(), ..video.clone() })
        .await
        .unwrap();
    tx.commit().await.unwrap();
    eventually("the new title to be searchable", async || ctx.find(None, &renamed).await.hits.len() == 1).await;
    assert!(ctx.find(None, &word).await.hits.is_empty());

    let mut tx = core.db.begin().await.unwrap();
    outbound::unindex(&mut *tx, "video", video.id).await.unwrap();
    tx.commit().await.unwrap();
    eventually("the unindexed video to disappear", async || ctx.find(None, &renamed).await.hits.is_empty()).await;

    // Users created before search existed are backfilled by the job an
    // administrator's reindex of the user kind starts.
    let users = SearchRequest {
        query: creator.handle.clone(),
        kind: SearchKind::SEARCH_KIND_USER.into(),
        ..Default::default()
    };
    assert!(ctx.search(None, &users).await.hits.is_empty(), "no event ever announced this user");
    let _: ReindexResponse = ctx
        .call(
            ADMIN,
            "Reindex",
            Some(&admin),
            &ReindexRequest { kind: SearchKind::SEARCH_KIND_USER.into(), ..Default::default() },
        )
        .await
        .unwrap();
    eventually("the user to be backfilled", async || {
        hit_ids(&ctx.search(None, &users).await.hits) == vec![creator.id.to_string()]
    })
    .await;

    // Identity's own events are consumed from the stream as well.
    ctx.app.identity.set_banned(creator.id, Some("spam")).await.unwrap();
    eventually("the suspended user's document to leave the index", async || {
        let held: Option<bool> =
            sqlx::query_scalar("SELECT doc IS NOT NULL FROM search_documents WHERE kind = 'user' AND doc_id = $1")
                .bind(creator.id)
                .fetch_optional(&core.db)
                .await
                .unwrap();
        held == Some(false)
    })
    .await;

    // Synonyms saved by an administrator are applied by the job.
    let (known, alias) = (unique("known"), unique("alias"));
    let mut tx = core.db.begin().await.unwrap();
    outbound::index(&mut *tx, doc("note", None, &known)).await.unwrap();
    tx.commit().await.unwrap();
    let _: SaveSynonymGroupResponse = ctx
        .call(
            ADMIN,
            "SaveSynonymGroup",
            Some(&admin),
            &SaveSynonymGroupRequest { words: vec![known.clone(), alias.clone()], ..Default::default() },
        )
        .await
        .unwrap();
    eventually("the synonym to find the note", async || ctx.find(None, &alias).await.hits.len() == 1).await;

    // The trending timer picks up what users search.
    for n in 0..3 {
        let user = ctx.app.user(&format!("searcher{n}")).await;
        ctx.find(Some(&user), &known).await;
    }
    eventually("the query to trend", async || ctx.trending(None).await.contains(&known)).await;

    core.shutdown.cancel();
}
