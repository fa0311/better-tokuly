//! What each user keeps on top of the catalogue — the reaction deck and the
//! recently used list — and how use becomes "popular".
mod common;

use common::*;
use tokuly_emoji::catalog;
use tokuly_proto::emoji::*;
use tokuly_testkit::TestUser;

fn texts(items: &[PickerItem]) -> Vec<&str> {
    items.iter().map(|item| item.emoji.as_str()).collect()
}

async fn deck(ctx: &Ctx, user: &TestUser) -> GetDeckResponse {
    ctx.call("", "GetDeck", Some(user), &GetDeckRequest::default()).await.unwrap()
}

async fn set_deck(ctx: &Ctx, user: &TestUser, emojis: &[&str]) -> Result<Vec<PickerItem>, String> {
    let request = SetDeckRequest { emojis: strings(emojis), ..Default::default() };
    match ctx.call::<_, SetDeckResponse>("", "SetDeck", Some(user), &request).await {
        Ok(response) => Ok(response.items),
        Err(error) => Err(error.reason),
    }
}

async fn recent(ctx: &Ctx, user: &TestUser) -> Vec<PickerItem> {
    let response: ListRecentEmojiResponse =
        ctx.call("", "ListRecentEmoji", Some(user), &ListRecentEmojiRequest::default()).await.unwrap();
    response.items
}

async fn popular(ctx: &Ctx) -> Vec<CustomEmoji> {
    let request = ListPopularEmojiRequest { limit: 100, ..Default::default() };
    let response: ListPopularEmojiResponse = ctx.call("", "ListPopularEmoji", None, &request).await.unwrap();
    response.emojis
}

#[tokio::test]
async fn everyone_starts_with_the_default_deck_and_makes_it_their_own() {
    let ctx = start().await;
    let (moderator, user) = (ctx.moderator().await, ctx.app.user("user").await);
    let name = unique("pinned");
    let emoji = ctx.add(&moderator, &name).await;
    let code = format!(":{name}:");

    let default = deck(&ctx, &user).await;
    assert!(default.is_default);
    assert_eq!(texts(&default.items), ["👍", "❤️", "😆", "🤔", "😮", "🎉", "💢", "😥", "😇", "🍮"]);
    assert!(default.items.iter().all(|item| !item.custom.is_set()));

    // Custom and Unicode emoji side by side, in the user's order, once each.
    let saved = set_deck(&ctx, &user, &["🎉", &code.to_uppercase(), "👨‍👩‍👧", &name, "🎉"]).await.unwrap();
    assert_eq!(texts(&saved), ["🎉", code.as_str(), "👨‍👩‍👧"]);
    assert_eq!(saved[1].custom.id, emoji.emoji.id);
    assert!(!saved[0].custom.is_set());
    let mine = deck(&ctx, &user).await;
    assert!(!mine.is_default);
    assert_eq!(texts(&mine.items), ["🎉", code.as_str(), "👨‍👩‍👧"]);

    assert_eq!(set_deck(&ctx, &user, &["hello world"]).await.unwrap_err(), "DECK_ITEM_INVALID");
    assert_eq!(set_deck(&ctx, &user, &["👍👍"]).await.unwrap_err(), "DECK_ITEM_INVALID");
    assert_eq!(set_deck(&ctx, &user, &[&format!(":{}:", unique("nothing"))]).await.unwrap_err(), "EMOJI_NOT_FOUND");
    let long: Vec<String> = (0..101).map(|_| "👍".to_owned()).collect();
    let long: Vec<&str> = long.iter().map(String::as_str).collect();
    assert_eq!(set_deck(&ctx, &user, &long).await.unwrap_err(), "DECK_TOO_LONG");
    assert_eq!(texts(&deck(&ctx, &user).await.items).len(), 3, "a refused deck changes nothing");

    // An emoji that left the catalogue stays pinned (without a picture), the
    // deck stays saveable, and the emoji is back when it returns.
    ctx.disable(&moderator, &emoji.emoji.id, "ちょっとおやすみ").await;
    let without = deck(&ctx, &user).await;
    assert_eq!(texts(&without.items), ["🎉", code.as_str(), "👨‍👩‍👧"]);
    assert!(!without.items[1].custom.is_set());
    let reordered = set_deck(&ctx, &user, &[&code, "🎉"]).await.unwrap();
    assert_eq!(texts(&reordered), [code.as_str(), "🎉"]);
    let other = ctx.app.user("other").await;
    assert_eq!(set_deck(&ctx, &other, &[&code]).await.unwrap_err(), "EMOJI_NOT_FOUND", "but it cannot be pinned anew");
    ctx.enable(&moderator, &emoji.emoji.id).await;
    assert_eq!(deck(&ctx, &user).await.items[0].custom.id, emoji.emoji.id);

    // An empty deck is a deck; resetting brings the default back.
    assert!(set_deck(&ctx, &user, &[]).await.unwrap().is_empty());
    let empty = deck(&ctx, &user).await;
    assert!(empty.items.is_empty() && !empty.is_default);
    let reset: ResetDeckResponse = ctx.call("", "ResetDeck", Some(&user), &ResetDeckRequest::default()).await.unwrap();
    assert_eq!(reset.items.len(), 10);
    assert!(deck(&ctx, &user).await.is_default);

    assert_eq!(ctx.reason("", "GetDeck", None, &GetDeckRequest::default()).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason("", "SetDeck", None, &SetDeckRequest::default()).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason("", "ResetDeck", None, &ResetDeckRequest::default()).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn used_emoji_move_to_the_top_of_recently_used() {
    let ctx = start().await;
    let (moderator, user, other) = (ctx.moderator().await, ctx.app.user("user").await, ctx.app.user("other").await);
    let (first, second) = (unique("first"), unique("second"));
    let first_id = ctx.add(&moderator, &first).await.emoji.id.clone();
    ctx.add(&moderator, &second).await;
    let (first_code, second_code) = (format!(":{first}:"), format!(":{second}:"));
    assert!(recent(&ctx, &user).await.is_empty());

    // The last one reported is the most recent; junk and unknown emoji are ignored.
    ctx.record_use(&user, &[&first_code, "👍", &format!(":{}:", unique("nothing")), "not an emoji", &second]).await;
    let list = recent(&ctx, &user).await;
    assert_eq!(texts(&list), [second_code.as_str(), "👍", first_code.as_str()]);
    assert_eq!(list[2].custom.id, first_id);
    assert!(!list[1].custom.is_set());
    // Using one again moves it up without listing it twice.
    ctx.record_use(&user, &[&first_code.to_uppercase(), &first_code]).await;
    assert_eq!(texts(&recent(&ctx, &user).await), [first_code.as_str(), second_code.as_str(), "👍"]);
    assert!(recent(&ctx, &other).await.is_empty(), "the list is one's own");

    // Other domains report uses through the Rust API.
    catalog::record_use(&ctx.app.core, other.id, &[second.to_uppercase(), "🎉".to_owned(), "junk!".to_owned()])
        .await
        .unwrap();
    assert_eq!(texts(&recent(&ctx, &other).await), ["🎉", second_code.as_str()]);

    let limited = ListRecentEmojiRequest { limit: 2, ..Default::default() };
    let limited: ListRecentEmojiResponse = ctx.call("", "ListRecentEmoji", Some(&user), &limited).await.unwrap();
    assert_eq!(texts(&limited.items), [first_code.as_str(), second_code.as_str()]);

    // Only the 64 most recent are kept.
    let many: Vec<String> = (0..40u32).map(|n| char::from_u32(0x1F600 + n).unwrap().to_string()).collect();
    let more: Vec<String> = (40..70u32).map(|n| char::from_u32(0x1F600 + n).unwrap().to_string()).collect();
    ctx.record_use(&user, &many.iter().map(String::as_str).collect::<Vec<_>>()).await;
    ctx.record_use(&user, &more.iter().map(String::as_str).collect::<Vec<_>>()).await;
    let kept = recent(&ctx, &user).await;
    assert_eq!(kept.len(), 64);
    assert_eq!(kept[0].emoji, more[29]);
    assert!(!texts(&kept).contains(&first_code.as_str()), "the oldest fell off");

    let too_many = RecordUseRequest { emojis: vec!["👍".to_owned(); 51], ..Default::default() };
    assert_eq!(ctx.reason("", "RecordUse", Some(&user), &too_many).await, "TOO_MANY_ITEMS");
    assert_eq!(ctx.reason("", "RecordUse", None, &RecordUseRequest::default()).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason("", "ListRecentEmoji", None, &ListRecentEmojiRequest::default()).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason("", "ClearRecentEmoji", None, &ClearRecentEmojiRequest::default()).await, "SIGN_IN_REQUIRED");

    let _: ClearRecentEmojiResponse =
        ctx.call("", "ClearRecentEmoji", Some(&user), &ClearRecentEmojiRequest::default()).await.unwrap();
    assert!(recent(&ctx, &user).await.is_empty());
    assert_eq!(recent(&ctx, &other).await.len(), 2);
}

/// The only test that runs the usage worker: its counters are one set for
/// the whole database, and a second test flushing them at the same time
/// would make "written yet?" a race.
#[tokio::test]
async fn use_is_counted_in_batches_and_popular_is_what_several_people_use() {
    let ctx = start().await;
    let db = &ctx.app.core.db;
    let moderator = ctx.moderator().await;
    let token = unique("pop");
    let (loved, liked, unused) = (format!("{token}_loved"), format!("{token}_liked"), format!("{token}_unused"));
    let loved_id = ctx.add(&moderator, &loved).await.emoji.id.clone();
    let liked_id = ctx.add(&moderator, &liked).await.emoji.id.clone();
    ctx.add(&moderator, &unused).await;
    let use_count = async |name: &str| ctx.get(Some(&moderator), name).await.unwrap().use_count;
    // Equal matches are ordered by name length, then name, while nothing is used.
    assert_eq!(positions(&ctx.search(&token, None).await, &[&liked, &loved, &unused]), [0, 1, 2]);

    // Three people use one emoji, one person the other; a use listed twice
    // in one report counts once.
    for n in 0..3 {
        let fan = ctx.app.user(&format!("fan{n}")).await;
        ctx.record_use(&fan, &[&loved, &loved, "👍"]).await;
    }
    let casual = ctx.app.user("casual").await;
    catalog::record_use(&ctx.app.core, casual.id, std::slice::from_ref(&liked)).await.unwrap();
    // Nothing is in the database until the worker runs.
    assert_eq!(use_count(&loved).await, 0);

    assert!(ctx.emoji.aggregate_usage().await.unwrap() >= 4);
    assert_eq!((use_count(&loved).await, use_count(&liked).await), (3, 1));
    assert_eq!(ctx.emoji.aggregate_usage().await.unwrap(), 0, "a batch is written once");
    assert_eq!(use_count(&loved).await, 3);
    // The exact count is for staff: it moves with every single use, and that
    // use may be one person's in a private chat.
    assert_eq!(ctx.get(None, &loved).await.unwrap().use_count, 0);
    assert_eq!(ctx.get(Some(&casual), &loved).await.unwrap().use_count, 0);

    // Popular is what several people use. One person's emoji is not in it…
    let list = popular(&ctx).await;
    assert!(named(&list, &loved).is_some());
    assert!(named(&list, &liked).is_none(), "one person using an emoji does not show");
    assert!(named(&list, &unused).is_none());
    let top: ListPopularEmojiResponse = ctx
        .call("", "ListPopularEmoji", None, &ListPopularEmojiRequest { limit: 1, ..Default::default() })
        .await
        .unwrap();
    assert_eq!(top.emojis.len(), 1);
    // …and among equal matches the popular emoji leads the search.
    assert_eq!(positions(&ctx.search(&token, None).await, &[&loved, &liked, &unused]), [0, 1, 2]);

    // Nor can an account push an emoji into the list alone: 20 uses a day
    // count, and they are still one person's.
    let spammer = ctx.app.user("spammer").await;
    for _ in 0..25 {
        ctx.record_use(&spammer, &[&liked]).await;
    }
    assert_eq!(ctx.emoji.aggregate_usage().await.unwrap(), 20);
    assert_eq!(use_count(&liked).await, 21);
    let today: (i64, i64) = sqlx::query_as("SELECT uses, users FROM emoji_usage_daily WHERE emoji_id = $1::uuid")
        .bind(&liked_id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(today, (21, 2));
    assert!(named(&popular(&ctx).await, &liked).is_none(), "two people are not a crowd either");
    // With a third person it is, and its use counts.
    ctx.record_use(&ctx.app.user("third").await, &[&liked]).await;
    ctx.emoji.aggregate_usage().await.unwrap();
    let order = positions(&popular(&ctx).await, &[&liked, &loved]);
    assert!(order[0] < order[1], "now that three people use it, its 22 uses lead");

    // Popularity looks at recent days only, older ones weighing less.
    sqlx::query(
        "INSERT INTO emoji_usage_daily (emoji_id, day, uses)
         VALUES ($1::uuid, (now() AT TIME ZONE 'Asia/Tokyo')::date - 3, 10), ($1::uuid, (now() AT TIME ZONE 'Asia/Tokyo')::date - 30, 1000)",
    )
    .bind(&loved_id)
    .execute(db)
    .await
    .unwrap();
    ctx.emoji.aggregate_usage().await.unwrap();
    let score: f64 = sqlx::query_scalar("SELECT popularity FROM emoji_stats WHERE emoji_id = $1::uuid")
        .bind(&loved_id)
        .fetch_one(db)
        .await
        .unwrap();
    let expected = 3.0 + 10.0 * 0.85f64.powi(3);
    assert!((score - expected).abs() < 1e-6, "{score} vs {expected}: a month-old day is out of the window");
    let order = positions(&popular(&ctx).await, &[&liked, &loved]);
    assert!(order[0] < order[1], "22 uses today outweigh 3 today and 10 three days ago");

    // A deleted emoji takes its numbers along, and uses reported for it
    // afterwards are dropped.
    ctx.record_use(&casual, &[&loved]).await;
    ctx.delete(&moderator, &loved_id).await;
    assert_eq!(ctx.emoji.aggregate_usage().await.unwrap(), 0);
    assert!(named(&popular(&ctx).await, &loved).is_none());
    ctx.emoji.prune().await.unwrap();
}
