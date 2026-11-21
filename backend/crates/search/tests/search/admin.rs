//! `SearchAdminService`: index statistics, reindexing, deleting documents,
//! the blocklist and synonyms.
use tokuly_proto::{common::PageRequest, search::*};
use tokuly_search::kinds::KINDS;
use tokuly_testkit::{RpcError, TestUser};

use crate::common::*;

async fn admin(ctx: &Ctx) -> TestUser {
    ctx.app.user_with_roles("admin", &["admin"]).await
}

async fn admin_call<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
    ctx: &Ctx,
    method: &str,
    user: Option<&TestUser>,
    request: &Req,
) -> Result<Resp, RpcError> {
    ctx.call(ADMIN, method, user, request).await
}

#[tokio::test]
async fn every_rpc_needs_the_admin_role() {
    let ctx = start().await;
    let (member, moderator) =
        (ctx.app.user("member").await, ctx.app.user_with_roles("moderator", &["moderator"]).await);
    let empty = serde_json::json!({});
    for method in [
        "GetIndexStats",
        "Reindex",
        "DeleteDocument",
        "ListBlockedTerms",
        "AddBlockedTerm",
        "RemoveBlockedTerm",
        "ListSynonymGroups",
        "SaveSynonymGroup",
        "DeleteSynonymGroup",
    ] {
        let anonymous = admin_call::<_, serde_json::Value>(&ctx, method, None, &empty).await.unwrap_err();
        assert_eq!(
            (anonymous.code.as_str(), anonymous.reason.as_str()),
            ("unauthenticated", "SIGN_IN_REQUIRED"),
            "{method}"
        );
        for user in [&member, &moderator] {
            let refused = admin_call::<_, serde_json::Value>(&ctx, method, Some(user), &empty).await.unwrap_err();
            assert_eq!(
                (refused.code.as_str(), refused.reason.as_str()),
                ("permission_denied", "FORBIDDEN"),
                "{method}"
            );
        }
    }
}

#[tokio::test]
async fn index_stats_show_the_engine_next_to_the_registry() {
    let ctx = start().await;
    let admin = admin(&ctx).await;
    let word = unique("stats");
    ctx.index(&[doc("clip", None, &word), doc("clip", None, &word), doc("playlist", None, &word)]).await;

    let stats: GetIndexStatsResponse =
        admin_call(&ctx, "GetIndexStats", Some(&admin), &GetIndexStatsRequest::default()).await.unwrap();
    assert!(stats.healthy);
    assert!(!stats.engine_version.is_empty());
    assert!(stats.database_size_bytes > 0);
    let kinds: Vec<i32> = stats.indexes.iter().map(|i| i.kind.to_i32()).collect();
    assert_eq!(kinds, (1..=12).collect::<Vec<_>>(), "one entry per kind, in SearchKind order");
    for (index, kind) in stats.indexes.iter().zip(KINDS) {
        assert_eq!(index.index_uid, format!("{}_{}", *PREFIX, kind.name));
        assert!(index.exists, "{} exists", index.index_uid);
    }
    let clips = stats.indexes.iter().find(|i| i.kind.as_known() == Some(SearchKind::SEARCH_KIND_CLIP)).unwrap();
    assert!(clips.indexed_documents >= 2 && clips.registered_documents >= 2, "{clips:?}");
    assert!(clips.last_changed_at.as_option().is_some());
    let playlists = stats.indexes.iter().find(|i| i.kind.as_known() == Some(SearchKind::SEARCH_KIND_PLAYLIST)).unwrap();
    assert!(playlists.indexed_documents >= 1 && playlists.registered_documents >= 1);
}

#[tokio::test]
async fn pending_changes_show_up_in_the_stats() {
    let ctx = start().await;
    let admin = admin(&ctx).await;
    // Keep every indexer away while the registry is ahead of the engine.
    let mut indexer_lock = ctx.app.core.db.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("search:indexer:{}", ctx.search.cfg.index_prefix))
        .execute(&mut *indexer_lock)
        .await
        .unwrap();
    ctx.publish(&doc("news", None, &unique("pending"))).await;

    let stats: GetIndexStatsResponse =
        admin_call(&ctx, "GetIndexStats", Some(&admin), &GetIndexStatsRequest::default()).await.unwrap();
    let news = stats.indexes.iter().find(|i| i.kind.as_known() == Some(SearchKind::SEARCH_KIND_NEWS)).unwrap();
    assert!(news.pending_documents >= 1 && news.registered_documents >= 1, "{news:?}");

    indexer_lock.rollback().await.unwrap();
    ctx.flush().await;
}

#[tokio::test]
async fn a_reindex_repairs_the_index_and_asks_the_owning_domain_to_republish() {
    let ctx = start().await;
    let admin = admin(&ctx).await;
    let word = unique("repair");
    let docs = [doc("playlist", None, &format!("{word} a")), doc("playlist", None, &format!("{word} b"))];
    ctx.index(&docs).await;
    let playlists =
        SearchRequest { query: word.clone(), kind: SearchKind::SEARCH_KIND_PLAYLIST.into(), ..Default::default() };
    assert_eq!(ctx.search(None, &playlists).await.hits.len(), 2);

    // The engine loses the documents behind the registry's back.
    let kind = KINDS.iter().find(|k| k.name == "playlist").unwrap();
    let ids: Vec<String> = docs.iter().map(|d| d.id.to_string()).collect();
    let engine = &ctx.app.core.search;
    let task = engine.index(ctx.search.index_uid(kind)).delete_documents(&ids).await.unwrap();
    assert!(task.wait_for_completion(engine, None, None).await.unwrap().is_success());
    assert!(ctx.search(None, &playlists).await.hits.is_empty());
    ctx.flush().await; // the registry believes everything is written
    assert!(ctx.search(None, &playlists).await.hits.is_empty());

    let response: ReindexResponse = admin_call(
        &ctx,
        "Reindex",
        Some(&admin),
        &ReindexRequest { kind: SearchKind::SEARCH_KIND_PLAYLIST.into(), ..Default::default() },
    )
    .await
    .unwrap();
    assert!(response.queued_documents >= 2);
    ctx.flush().await;
    assert_eq!(ctx.search(None, &playlists).await.hits.len(), 2);

    // The owning domain is asked to publish its documents again.
    let requests = ctx.app.events_by("search.reindex.requested", admin.id).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].data, serde_json::json!({ "kind": "playlist", "requested_by": admin.id }));
}

#[tokio::test]
async fn reindex_and_delete_validate_their_target() {
    let ctx = start().await;
    let admin = admin(&ctx).await;
    let cases: Vec<(&str, serde_json::Value, &str)> = vec![
        ("Reindex", serde_json::json!({}), "KIND_REQUIRED"),
        ("Reindex", serde_json::json!({ "kind": 99 }), "KIND_UNKNOWN"),
        ("DeleteDocument", serde_json::json!({ "id": uuid::Uuid::now_v7() }), "KIND_REQUIRED"),
        ("DeleteDocument", serde_json::json!({ "kind": 99, "id": uuid::Uuid::now_v7() }), "KIND_UNKNOWN"),
        ("DeleteDocument", serde_json::json!({ "kind": "SEARCH_KIND_VIDEO", "id": "nope" }), "INVALID_ID"),
        ("DeleteDocument", serde_json::json!({ "kind": "SEARCH_KIND_VIDEO" }), "INVALID_ID"),
    ];
    for (method, request, reason) in cases {
        let error = admin_call::<_, serde_json::Value>(&ctx, method, Some(&admin), &request).await.unwrap_err();
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("invalid_argument", reason), "{method} {request}");
    }
    // Reindexing users is accepted as well (the search domain republishes them itself).
    let _: ReindexResponse = admin_call(
        &ctx,
        "Reindex",
        Some(&admin),
        &ReindexRequest { kind: SearchKind::SEARCH_KIND_USER.into(), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(ctx.app.events_by("search.reindex.requested", admin.id).await[0].data["kind"], "user");
}

#[tokio::test]
async fn a_deleted_document_stays_out_until_it_is_published_again() {
    let ctx = start().await;
    let admin = admin(&ctx).await;
    let word = unique("delete");
    let (target, bystander) =
        (doc("short", None, &format!("{word} target")), doc("short", None, &format!("{word} bystander")));
    let publication = event(tokuly_core::outbound::SEARCH_TOPIC, upsert_of(&target));
    ctx.deliver(publication.clone()).await;
    ctx.index(std::slice::from_ref(&bystander)).await;
    assert_eq!(ctx.find(None, &word).await.hits.len(), 2);

    let request = DeleteDocumentRequest {
        kind: SearchKind::SEARCH_KIND_SHORT.into(),
        id: target.id.to_string(),
        ..Default::default()
    };
    let deleted: DeleteDocumentResponse = admin_call(&ctx, "DeleteDocument", Some(&admin), &request).await.unwrap();
    assert!(deleted.existed);
    ctx.flush().await;
    assert_eq!(hit_ids(&ctx.find(None, &word).await.hits), vec![bystander.id.to_string()]);

    // Deleting it again is harmless, and a redelivery of the old
    // publication does not undo the deletion.
    let again: DeleteDocumentResponse = admin_call(&ctx, "DeleteDocument", Some(&admin), &request).await.unwrap();
    assert!(!again.existed);
    ctx.deliver(publication).await;
    ctx.flush().await;
    assert_eq!(ctx.find(None, &word).await.hits.len(), 1);

    // The owning domain publishing it anew brings it back.
    ctx.index(std::slice::from_ref(&target)).await;
    assert_eq!(ctx.find(None, &word).await.hits.len(), 2);
}

#[tokio::test]
async fn deleting_reaches_the_engine_even_when_the_registry_never_held_the_document() {
    let ctx = start().await;
    let admin = admin(&ctx).await;
    let word = unique("stray");
    // A document that is in the index although the registry knows nothing of it.
    let stray = doc("clip", None, &word);
    let kind = KINDS.iter().find(|k| k.name == "clip").unwrap();
    let engine = &ctx.app.core.search;
    let body = tokuly_search::document::IndexDoc::from_search_doc(stray.clone()).to_index_json(kind);
    let task = engine.index(ctx.search.index_uid(kind)).add_or_replace(&[body], Some("id")).await.unwrap();
    assert!(task.wait_for_completion(engine, None, None).await.unwrap().is_success());
    assert_eq!(ctx.find(None, &word).await.hits.len(), 1);

    let deleted: DeleteDocumentResponse = admin_call(
        &ctx,
        "DeleteDocument",
        Some(&admin),
        &DeleteDocumentRequest {
            kind: SearchKind::SEARCH_KIND_CLIP.into(),
            id: stray.id.to_string(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(!deleted.existed);
    ctx.flush().await;
    assert!(ctx.find(None, &word).await.hits.is_empty());
}

#[tokio::test]
async fn blocked_terms_are_normalized_listed_and_removed() {
    let ctx = start().await;
    let admin = admin(&ctx).await;
    let stem = unique("term");
    let add = async |term: &str, note: &str| {
        admin_call::<_, AddBlockedTermResponse>(
            &ctx,
            "AddBlockedTerm",
            Some(&admin),
            &AddBlockedTermRequest { term: term.to_owned(), note: note.to_owned(), ..Default::default() },
        )
        .await
    };

    let added =
        add(&format!("  {}　ＢＡＤ ", stem.to_uppercase()), "abuse").await.unwrap().term.as_option().cloned().unwrap();
    assert_eq!(added.term, format!("{stem} bad"), "lowercase, width-folded, single spaces");
    assert_eq!(added.note, "abuse");
    assert_eq!(added.added_by.as_option().unwrap().id, admin.id.to_string());
    assert!(added.created_at.as_option().is_some());
    // Adding it again only updates the note.
    let updated =
        add(&format!("{stem} bad"), "abuse, reported twice").await.unwrap().term.as_option().cloned().unwrap();
    assert_eq!((updated.note.as_str(), updated.created_at == added.created_at), ("abuse, reported twice", true));
    for n in 0..4 {
        add(&format!("{stem} {n}"), "").await.unwrap();
    }

    // Listed alphabetically, in pages (other tests' terms are in there too).
    let mut listed: Vec<BlockedTerm> = Vec::new();
    let mut cursor = String::new();
    loop {
        let response: ListBlockedTermsResponse = admin_call(
            &ctx,
            "ListBlockedTerms",
            Some(&admin),
            &ListBlockedTermsRequest {
                page: Some(PageRequest { limit: 2, cursor, ..Default::default() }).into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(response.terms.len() <= 2);
        listed.extend(response.terms);
        let info = response.page.as_option().unwrap();
        if !info.has_more {
            break;
        }
        cursor = info.next_cursor.clone();
    }
    let all: Vec<&str> = listed.iter().map(|t| t.term.as_str()).collect();
    assert!(all.is_sorted(), "{all:?}");
    let mine: Vec<&str> = all.iter().copied().filter(|t| t.starts_with(&stem)).collect();
    let expected: Vec<String> = ["0", "1", "2", "3", "bad"].iter().map(|suffix| format!("{stem} {suffix}")).collect();
    assert_eq!(mine, expected);
    let bad = listed.iter().find(|t| t.term == format!("{stem} bad")).unwrap();
    assert_eq!(bad.note, "abuse, reported twice");

    // Removal, by any spelling of the term.
    let _: RemoveBlockedTermResponse = admin_call(
        &ctx,
        "RemoveBlockedTerm",
        Some(&admin),
        &RemoveBlockedTermRequest { term: format!("{} BAD", stem.to_uppercase()), ..Default::default() },
    )
    .await
    .unwrap();
    let gone = admin_call::<_, RemoveBlockedTermResponse>(
        &ctx,
        "RemoveBlockedTerm",
        Some(&admin),
        &RemoveBlockedTermRequest { term: format!("{stem} bad"), ..Default::default() },
    )
    .await
    .unwrap_err();
    assert_eq!((gone.code.as_str(), gone.reason.as_str()), ("not_found", "TERM_NOT_FOUND"));

    // Validation.
    assert_eq!(add("   ", "").await.unwrap_err().reason, "TERM_INVALID");
    assert_eq!(add(&"あ".repeat(65), "").await.unwrap_err().reason, "TERM_INVALID");
    assert_eq!(add(&format!("{stem} ok"), &"n".repeat(201)).await.unwrap_err().reason, "NOTE_TOO_LONG");
    let bad_cursor = admin_call::<_, ListBlockedTermsResponse>(
        &ctx,
        "ListBlockedTerms",
        Some(&admin),
        &ListBlockedTermsRequest {
            page: Some(PageRequest { cursor: "garbage".into(), ..Default::default() }).into(),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(bad_cursor.reason, "INVALID_CURSOR");
}

#[tokio::test]
async fn synonym_groups_make_words_find_each_other() {
    let ctx = start().await;
    let admin = admin(&ctx).await;
    let (stamp, sticker, seal) = (unique("stamp"), unique("sticker"), unique("seal"));
    let pack = doc("stamp_pack", None, &format!("{stamp} collection"));
    ctx.index(std::slice::from_ref(&pack)).await;
    assert!(ctx.find(None, &sticker).await.hits.is_empty());
    let save = async |id: &str, words: Vec<String>| {
        admin_call::<_, SaveSynonymGroupResponse>(
            &ctx,
            "SaveSynonymGroup",
            Some(&admin),
            &SaveSynonymGroupRequest { id: id.to_owned(), words, ..Default::default() },
        )
        .await
    };
    // What the `search.synonyms.apply` job does in a worker.
    let apply = async || {
        ctx.search.ensure_indexes().await.unwrap();
        ctx.search.wait_until_idle(std::time::Duration::from_secs(60)).await.unwrap();
    };

    let group = save("", vec![stamp.to_uppercase(), sticker.clone(), sticker.clone()])
        .await
        .unwrap()
        .group
        .as_option()
        .cloned()
        .unwrap();
    assert_eq!(group.words, vec![stamp.clone(), sticker.clone()], "normalized and deduplicated");
    assert!(!group.id.is_empty() && group.updated_at.as_option().is_some());
    apply().await;
    assert_eq!(
        hit_ids(&ctx.find(None, &sticker).await.hits),
        vec![pack.id.to_string()],
        "a synonym finds the document"
    );
    assert!(ctx.find(None, &seal).await.hits.is_empty());

    // Replacing the words of the group.
    let updated = save(&group.id, vec![stamp.clone(), seal.clone()]).await.unwrap().group.as_option().cloned().unwrap();
    assert_eq!((updated.id.as_str(), updated.words.clone()), (group.id.as_str(), vec![stamp.clone(), seal.clone()]));
    apply().await;
    assert_eq!(ctx.find(None, &seal).await.hits.len(), 1);
    assert!(ctx.find(None, &sticker).await.hits.is_empty());

    let listed: ListSynonymGroupsResponse =
        admin_call(&ctx, "ListSynonymGroups", Some(&admin), &ListSynonymGroupsRequest::default()).await.unwrap();
    assert_eq!(listed.groups.iter().find(|g| g.id == group.id).unwrap().words, vec![stamp.clone(), seal.clone()]);

    // Deleting the group takes the synonyms away.
    let _: DeleteSynonymGroupResponse = admin_call(
        &ctx,
        "DeleteSynonymGroup",
        Some(&admin),
        &DeleteSynonymGroupRequest { id: group.id.clone(), ..Default::default() },
    )
    .await
    .unwrap();
    apply().await;
    assert!(ctx.find(None, &seal).await.hits.is_empty());
    assert_eq!(ctx.find(None, &stamp).await.hits.len(), 1);

    // Validation.
    let missing = admin_call::<_, DeleteSynonymGroupResponse>(
        &ctx,
        "DeleteSynonymGroup",
        Some(&admin),
        &DeleteSynonymGroupRequest { id: group.id.clone(), ..Default::default() },
    )
    .await
    .unwrap_err();
    assert_eq!((missing.code.as_str(), missing.reason.as_str()), ("not_found", "SYNONYM_GROUP_NOT_FOUND"));
    assert_eq!(save(&group.id, vec![stamp.clone(), seal.clone()]).await.unwrap_err().reason, "SYNONYM_GROUP_NOT_FOUND");
    assert_eq!(save("not-an-id", vec![stamp.clone(), seal.clone()]).await.unwrap_err().reason, "INVALID_ID");
    assert_eq!(save("", vec![stamp.clone()]).await.unwrap_err().reason, "SYNONYMS_INVALID");
    assert_eq!(save("", vec![stamp.clone(), "  ".to_owned()]).await.unwrap_err().reason, "SYNONYMS_INVALID");
    let many: Vec<String> = (0..21).map(|n| format!("{stamp}{n}")).collect();
    assert_eq!(save("", many).await.unwrap_err().reason, "SYNONYMS_INVALID");
}
