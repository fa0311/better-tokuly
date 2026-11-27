//! The catalogue as clients and other domains read it: the whole set, the
//! differences since a version, the announcement of new versions, search,
//! the newest list, emoji pages, and the cross-domain Rust API.
mod common;

use common::*;
use tokuly_emoji::catalog;
use tokuly_proto::emoji::{__buffa::oneof::emoji_event, *};

/// The version at which an emoji last changed in the catalogue.
async fn catalog_version(ctx: &Ctx, emoji_id: &str) -> i64 {
    sqlx::query_scalar("SELECT catalog_version FROM emojis WHERE id = $1::uuid")
        .bind(emoji_id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_catalogue_lists_approved_emoji_and_their_categories() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let category = unique("どうぶつ");
    ctx.create_category(&moderator, &category).await;
    let name = unique("blobcat");
    let request = AddEmojiRequest {
        category: category.clone(),
        aliases: strings(&["Cat", "ねこ"]),
        license: "CC0".to_owned(),
        sensitive: true,
        local_only: true,
        ..ctx.add_request(&moderator, &name).await
    };
    let added = ctx.add_with(&moderator, request).await;
    assert_eq!(added.state, EmojiState::EMOJI_STATE_APPROVED);

    let before = ctx.catalog().await;
    let listed = named(&before.emojis, &name).expect("an added emoji is in the catalogue");
    assert_eq!(listed.id, added.emoji.id);
    assert_eq!((listed.category.as_str(), listed.aliases.clone()), (category.as_str(), strings(&["cat", "ねこ"])));
    assert!(listed.sensitive && listed.local_only && !listed.animated);
    assert!(
        !listed.image.url.is_empty() && !listed.image.variants.is_empty(),
        "clients get the stills to show it small"
    );
    assert!(listed.added_at.is_set());
    let shelf = before.categories.iter().find(|c| c.name == category).expect("the category is listed");
    assert_eq!(shelf.emoji_count, 1);
    assert!(before.version >= 1);
    // The catalogue is ordered by name.
    let names: Vec<&str> = before.emojis.iter().map(|emoji| emoji.name.as_str()).collect();
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));

    // What waits for review is not in it.
    let pending = ctx.submit(&author, &unique("waiting")).await;
    assert!(named(&ctx.catalog().await.emojis, &pending.emoji.name).is_none());
    ctx.approve(&moderator, &pending.emoji.id).await;
    let after = ctx.catalog().await;
    assert!(named(&after.emojis, &pending.emoji.name).is_some());
    assert!(after.version > before.version, "every change moves the version forward");

    let categories: ListCategoriesResponse =
        ctx.call("", "ListCategories", None, &ListCategoriesRequest::default()).await.unwrap();
    assert!(categories.categories.iter().any(|c| c.name == category && c.emoji_count == 1));
    assert!(categories.uncategorized_count >= 1);
}

#[tokio::test]
async fn animated_emoji_keep_their_animation_and_offer_stills() {
    let ctx = start().await;
    let moderator = ctx.moderator().await;
    let name = unique("party");
    let request =
        AddEmojiRequest { asset_id: ctx.animation(&moderator).await, ..ctx.add_request(&moderator, &name).await };
    let asset_id = request.asset_id.clone();
    let added = ctx.add_with(&moderator, request).await;
    assert!(added.emoji.animated);
    let animation_url: String = sqlx::query_scalar("SELECT url FROM media_assets WHERE id = $1::uuid")
        .bind(&asset_id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(animation_url.contains("/anim/"));
    assert_eq!(added.emoji.image.url, animation_url, "the main URL is the animated rendition");
    assert!(added.emoji.image.variants.iter().all(|variant| variant.url.contains("/img/")), "the variants are stills");
    assert!(named(&ctx.catalog().await.emojis, &name).unwrap().animated);
}

#[tokio::test]
async fn clients_catch_up_with_what_changed_since_their_version() {
    let ctx = start().await;
    let moderator = ctx.moderator().await;
    let category = unique("catchup");
    ctx.create_category(&moderator, &category).await;
    // Make sure the catalogue has a history before the client's first sync.
    ctx.add(&moderator, &unique("warmup")).await;
    let synced = ctx.catalog().await.version;

    let (kept_name, disabled_name, deleted_name) = (unique("kept"), unique("disabled"), unique("deleted"));
    let kept = ctx
        .add_with(
            &moderator,
            AddEmojiRequest { category: category.clone(), ..ctx.add_request(&moderator, &kept_name).await },
        )
        .await;
    let kept_id = kept.emoji.id.clone();
    let disabled = ctx
        .add_with(
            &moderator,
            AddEmojiRequest { category: category.clone(), ..ctx.add_request(&moderator, &disabled_name).await },
        )
        .await;
    let disabled_id = disabled.emoji.id.clone();
    let deleted = ctx
        .add_with(
            &moderator,
            AddEmojiRequest { category: category.clone(), ..ctx.add_request(&moderator, &deleted_name).await },
        )
        .await;
    let deleted_id = deleted.emoji.id.clone();

    let added = ctx.changes(synced).await;
    assert!(!added.full && added.version > synced);
    for name in [&kept_name, &disabled_name, &deleted_name] {
        assert!(named(&added.upserted, name).is_some(), "{name} was added since {synced}");
    }
    let added_category = added
        .categories
        .iter()
        .find(|candidate| candidate.name == category)
        .expect("the delta includes the category whose emoji count changed");
    assert_eq!(added_category.emoji_count, 3);
    let synced = added.version;

    ctx.disable(&moderator, &disabled_id, "調整中だよ").await;
    ctx.delete(&moderator, &deleted_id).await;
    let renamed = unique("renamed");
    ctx.update(
        &moderator,
        UpdateEmojiRequest { emoji_id: kept_id.clone(), name: Some(renamed.clone()), ..Default::default() },
    )
    .await;

    let changed = ctx.changes(synced).await;
    assert!(!changed.full);
    assert!(changed.removed_ids.contains(&disabled_id) && changed.removed_ids.contains(&deleted_id));
    assert!(named(&changed.upserted, &disabled_name).is_none() && named(&changed.upserted, &deleted_name).is_none());
    let moved = named(&changed.upserted, &renamed).expect("a renamed emoji is an upsert");
    assert_eq!(moved.id, kept_id, "a rename keeps the ID clients key their copy by");

    // An emoji that comes back is an upsert again, no longer a removal.
    ctx.enable(&moderator, &disabled_id).await;
    let back = ctx.changes(synced).await;
    assert!(named(&back.upserted, &disabled_name).is_some());
    assert!(!back.removed_ids.contains(&disabled_id) && back.removed_ids.contains(&deleted_id));

    // Versions the server cannot compare with get the whole catalogue.
    let unknown = ctx.changes(back.version + 1_000_000).await;
    assert!(unknown.full && named(&unknown.upserted, &renamed).is_some() && unknown.removed_ids.is_empty());
    let first_sync = ctx.changes(0).await;
    assert!(first_sync.full && first_sync.upserted.len() >= 2);
    let request = GetCatalogChangesRequest { since_version: -1, ..Default::default() };
    assert_eq!(ctx.reason("", "GetCatalogChanges", None, &request).await, "VERSION_INVALID");
}

#[tokio::test]
async fn a_new_version_is_announced_on_the_public_topic() {
    let ctx = start().await;
    let moderator = ctx.moderator().await;
    let mut topic = ctx.app.listen_topic(catalog::TOPIC).await;
    let added = ctx.add(&moderator, &unique("announced")).await;
    let version = catalog_version(&ctx, &added.emoji.id).await;
    assert!(version >= 1);
    // Announcements are bundled (about one a second), so the one that
    // covers this change may carry a later version.
    let announced = next_matching(&mut topic, |frame| match emoji_event(frame)? {
        emoji_event::Event::CatalogChanged(changed) if changed.version >= version => {
            Some((frame.topic.clone(), changed.version))
        }
        _ => None,
    })
    .await
    .expect("the change is announced");
    assert_eq!(announced.0, "emoji:catalog");
    assert!(ctx.changes(version - 1).await.version >= version);
}

#[tokio::test]
async fn a_change_that_was_never_published_is_caught_up_by_maintenance() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    ctx.add(&moderator, &unique("warmup")).await;
    let pending = ctx.submit(&author, &unique("missed")).await;
    let mut topic = ctx.app.listen_topic(catalog::TOPIC).await;

    // A process that committed a change and stopped before telling anyone.
    let version: i64 = sqlx::query_scalar(
        "WITH next AS (UPDATE emoji_catalog SET version = version + 1 RETURNING version)
         UPDATE emojis SET state = 2, approved_at = now(), catalog_version = (SELECT version FROM next)
         WHERE id = $1::uuid
         RETURNING catalog_version",
    )
    .bind(&pending.emoji.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();

    ctx.emoji.maintain().await.unwrap();
    let catalogue = ctx.catalog().await;
    assert!(catalogue.version >= version);
    assert!(named(&catalogue.emojis, &pending.emoji.name).is_some(), "the cache follows the database");
    let announced = next_matching(&mut topic, |frame| match emoji_event(frame)? {
        emoji_event::Event::CatalogChanged(changed) if changed.version >= version => Some(changed.version),
        _ => None,
    })
    .await;
    assert!(announced.is_some(), "the missed version is announced");
}

#[tokio::test]
async fn search_puts_exact_and_prefix_matches_first() {
    let ctx = start().await;
    let moderator = ctx.moderator().await;
    let token = unique("q");
    let category = unique("さがす");
    ctx.create_category(&moderator, &category).await;

    let exact = token.clone();
    let prefixed = format!("{token}_jam");
    let inside = format!("big_{token}");
    let (by_alias, by_alias_prefix, by_kana) = (unique("alias"), unique("aprefix"), unique("kana"));
    ctx.add(&moderator, &inside).await;
    ctx.add(&moderator, &prefixed).await;
    ctx.add(&moderator, &exact).await;
    let aliased = AddEmojiRequest { aliases: vec![token.clone()], ..ctx.add_request(&moderator, &by_alias).await };
    ctx.add_with(&moderator, aliased).await;
    let alias_prefixed = AddEmojiRequest {
        aliases: vec![format!("{token}zzz")],
        category: category.clone(),
        ..ctx.add_request(&moderator, &by_alias_prefix).await
    };
    ctx.add_with(&moderator, alias_prefixed).await;
    let kana =
        AddEmojiRequest { aliases: vec![format!("ネコ{token}")], ..ctx.add_request(&moderator, &by_kana).await };
    ctx.add_with(&moderator, kana).await;

    let found = ctx.search(&token, None).await;
    let order = positions(&found, &[&exact, &by_alias, &prefixed, &by_alias_prefix, &inside, &by_kana]);
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "exact name, exact alias, prefixes, then substrings: {order:?}"
    );

    // Case, colons and the kind of kana do not matter.
    assert_eq!(ctx.search(&format!(":{}:", token.to_uppercase()), None).await[0].name, exact);
    assert_eq!(ctx.search(&format!("ねこ{token}"), None).await[0].name, by_kana);
    assert_eq!(ctx.search(&format!("ﾈｺ{token}"), None).await[0].name, by_kana);

    // Narrowed to a category, and a category listed without a query.
    let narrowed = ctx.search(&token, Some(&category)).await;
    assert_eq!(narrowed.iter().map(|emoji| emoji.name.as_str()).collect::<Vec<_>>(), [by_alias_prefix.as_str()]);
    assert_eq!(ctx.search("", Some(&category)).await.len(), 1);
    assert!(ctx.search("", Some("")).await.iter().any(|emoji| emoji.name == exact), "the empty category is \"none\"");
    // The category's name finds its emoji too.
    assert!(ctx.search(&category, None).await.iter().any(|emoji| emoji.name == by_alias_prefix));

    assert!(ctx.search("", None).await.is_empty(), "an empty query finds nothing");
    assert!(ctx.search(&unique("nothing"), None).await.is_empty());
    let limited = SearchEmojiRequest { query: token.clone(), limit: 2, ..Default::default() };
    let response: SearchEmojiResponse = ctx.call("", "SearchEmoji", None, &limited).await.unwrap();
    assert_eq!(response.emojis.len(), 2);
    let too_long = SearchEmojiRequest { query: "q".repeat(65), ..Default::default() };
    assert_eq!(ctx.reason("", "SearchEmoji", None, &too_long).await, "QUERY_TOO_LONG");
}

#[tokio::test]
async fn the_newest_emoji_are_listed_first() {
    let ctx = start().await;
    let moderator = ctx.moderator().await;
    let (first, second, third) = (unique("first"), unique("second"), unique("third"));
    for name in [&first, &second, &third] {
        ctx.add(&moderator, name).await;
    }

    // Walk the pages; other tests add emoji meanwhile, so only the order of
    // these three is checked.
    let mut seen: Vec<CustomEmoji> = Vec::new();
    let mut cursor = String::new();
    for _ in 0..200 {
        let request = ListNewestEmojiRequest { page: page(2, &cursor), ..Default::default() };
        let response: ListNewestEmojiResponse = ctx.call("", "ListNewestEmoji", None, &request).await.unwrap();
        assert!(response.emojis.len() <= 2);
        seen.extend(response.emojis);
        if !response.page.has_more || named(&seen, &first).is_some() {
            break;
        }
        cursor = response.page.next_cursor.clone();
    }
    let order = positions(&seen, &[&third, &second, &first]);
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
    let ids: std::collections::HashSet<&str> = seen.iter().map(|emoji| emoji.id.as_str()).collect();
    assert_eq!(ids.len(), seen.len(), "no emoji appears on two pages");
    let bad_cursor = ListNewestEmojiRequest { page: page(2, "!!!"), ..Default::default() };
    assert_eq!(ctx.reason("", "ListNewestEmoji", None, &bad_cursor).await, "INVALID_CURSOR");
}

#[tokio::test]
async fn an_emoji_page_shows_author_and_licence_to_everyone_and_reviews_to_few() {
    let ctx = start().await;
    let (moderator, author, stranger) =
        (ctx.moderator().await, ctx.app.user("author").await, ctx.app.user("stranger").await);
    let name = unique("page");
    let request = SubmitEmojiRequest {
        license: "CC BY 4.0 / えがいたひと".to_owned(),
        ..ctx.submit_request(&author, &name).await
    };
    let response: SubmitEmojiResponse = ctx.call("Submission", "SubmitEmoji", Some(&author), &request).await.unwrap();
    let id = response.emoji.emoji.id.clone();

    // Waiting for review: between the author and the moderators.
    assert_eq!(ctx.get(None, &name).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    assert_eq!(ctx.get(Some(&stranger), &id).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    let mine = ctx.get(Some(&author), &id).await.unwrap();
    assert_eq!(mine.state, EmojiState::EMOJI_STATE_PENDING);
    assert!(mine.can_edit && mine.history.is_empty());
    let reviewed = ctx.get(Some(&moderator), &name).await.unwrap();
    assert_eq!(reviewed.history.len(), 1);
    assert_eq!(reviewed.history[0].action, ReviewAction::REVIEW_ACTION_SUBMITTED);
    assert_eq!(reviewed.history[0].actor.id, author.id.to_string());
    assert!(!reviewed.can_edit);

    ctx.approve(&moderator, &id).await;
    for reference in [name.clone(), format!(":{name}:"), name.to_uppercase(), id.clone()] {
        let public = ctx.get(None, &reference).await.unwrap();
        assert_eq!(public.emoji.name, name);
        assert_eq!(public.state, EmojiState::EMOJI_STATE_APPROVED);
        assert_eq!(public.author.id, author.id.to_string());
        assert_eq!(public.license, "CC BY 4.0 / えがいたひと");
        assert!(public.history.is_empty() && !public.reviewed_at.is_set() && !public.can_edit);
    }
    assert!(ctx.get(Some(&author), &id).await.unwrap().reviewed_at.is_set());
    assert_eq!(ctx.get(None, &unique("nobody")).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    assert_eq!(ctx.get(None, "not a name!").await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    assert_eq!(ctx.get(None, &uuid::Uuid::now_v7().to_string()).await.unwrap_err().reason, "EMOJI_NOT_FOUND");

    // The author's emoji, newest first.
    let second = ctx.submit(&author, &unique("page")).await;
    ctx.approve(&moderator, &second.emoji.id).await;
    ctx.submit(&author, &unique("unpublished")).await;
    let request = ListAuthorEmojiRequest { user_id: author.id.to_string(), page: page(1, ""), ..Default::default() };
    let newest: ListAuthorEmojiResponse = ctx.call("", "ListAuthorEmoji", None, &request).await.unwrap();
    assert_eq!(newest.emojis.len(), 1);
    assert_eq!(newest.emojis[0].id, second.emoji.id);
    assert!(newest.page.has_more);
    let request = ListAuthorEmojiRequest { page: page(10, &newest.page.next_cursor), ..request };
    let older: ListAuthorEmojiResponse = ctx.call("", "ListAuthorEmoji", None, &request).await.unwrap();
    assert_eq!(older.emojis.iter().map(|emoji| emoji.id.as_str()).collect::<Vec<_>>(), [id.as_str()]);
    assert!(!older.page.has_more);
    let request = ListAuthorEmojiRequest { user_id: "nope".to_owned(), ..Default::default() };
    assert_eq!(ctx.reason("", "ListAuthorEmoji", None, &request).await, "INVALID_ID");
}

#[tokio::test]
async fn other_domains_resolve_names_from_the_catalogue() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let name = unique("resolve");
    let added = ctx.add(&moderator, &name).await;
    let pending = ctx.submit(&author, &unique("pending")).await;

    assert!(catalog::exists(core, &name).await.unwrap());
    assert!(catalog::exists(core, &format!(":{}:", name.to_uppercase())).await.unwrap());
    assert!(!catalog::exists(core, &pending.emoji.name).await.unwrap(), "only approved emoji exist for other domains");
    assert!(!catalog::exists(core, "").await.unwrap());

    let shouted = name.to_uppercase();
    let asked = vec![name.clone(), shouted.clone(), pending.emoji.name.clone(), unique("nothing")];
    let images = catalog::resolve(core, &asked).await.unwrap();
    assert_eq!(images.len(), 2, "unknown and unapproved names are absent");
    assert_eq!(images[&name].url, added.emoji.image.url);
    assert_eq!(images[&shouted], images[&name], "results are keyed by the name as it was asked for");
    let entries = catalog::lookup(core, &asked).await.unwrap();
    assert_eq!(entries[&name].id, added.emoji.id);
    assert!(catalog::resolve(core, &[]).await.unwrap().is_empty());

    // A second app instance (as another request handler would be) sees
    // every change at once.
    let other = start().await;
    let renamed = unique("renamed");
    ctx.update(
        &moderator,
        UpdateEmojiRequest { emoji_id: added.emoji.id.clone(), name: Some(renamed.clone()), ..Default::default() },
    )
    .await;
    assert!(!catalog::exists(&other.app.core, &name).await.unwrap());
    assert!(catalog::exists(&other.app.core, &renamed).await.unwrap());
    ctx.disable(&moderator, &added.emoji.id, "おやすみ中").await;
    assert!(!catalog::exists(&other.app.core, &renamed).await.unwrap());
    assert!(catalog::resolve(&other.app.core, std::slice::from_ref(&renamed)).await.unwrap().is_empty());
    other.enable(&moderator, &added.emoji.id).await;
    assert!(catalog::exists(core, &renamed).await.unwrap());
}
