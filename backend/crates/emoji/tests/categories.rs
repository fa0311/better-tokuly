//! The managed list of categories. One test walks through it from an empty
//! list: the list is one for the whole database, and `ReorderCategories`
//! names all of it, so nothing else in this file creates categories.
mod common;

use common::*;
use futures::future::join_all;
use tokuly_proto::emoji::*;
use tokuly_testkit::TestUser;

/// `(name, emoji_count)` of every category, in display order.
fn shelf(categories: &[Category]) -> Vec<(&str, i32)> {
    categories.iter().map(|category| (category.name.as_str(), category.emoji_count)).collect()
}

async fn create(ctx: &Ctx, moderator: &TestUser, name: &str) -> Result<Vec<Category>, String> {
    let request = CreateCategoryRequest { name: name.to_owned(), ..Default::default() };
    match ctx.call::<_, CreateCategoryResponse>("Admin", "CreateCategory", Some(moderator), &request).await {
        Ok(response) => Ok(response.categories),
        Err(error) => Err(error.reason),
    }
}

async fn rename(ctx: &Ctx, moderator: &TestUser, name: &str, new_name: &str) -> Result<Vec<Category>, String> {
    let request = RenameCategoryRequest { name: name.to_owned(), new_name: new_name.to_owned(), ..Default::default() };
    match ctx.call::<_, RenameCategoryResponse>("Admin", "RenameCategory", Some(moderator), &request).await {
        Ok(response) => Ok(response.categories),
        Err(error) => Err(error.reason),
    }
}

async fn reorder(ctx: &Ctx, moderator: &TestUser, names: &[&str]) -> Result<Vec<Category>, String> {
    let request = ReorderCategoriesRequest { names: strings(names), ..Default::default() };
    match ctx.call::<_, ReorderCategoriesResponse>("Admin", "ReorderCategories", Some(moderator), &request).await {
        Ok(response) => Ok(response.categories),
        Err(error) => Err(error.reason),
    }
}

async fn delete(ctx: &Ctx, moderator: &TestUser, name: &str, move_to: &str) -> Result<Vec<Category>, String> {
    let request = DeleteCategoryRequest { name: name.to_owned(), move_to: move_to.to_owned(), ..Default::default() };
    match ctx.call::<_, DeleteCategoryResponse>("Admin", "DeleteCategory", Some(moderator), &request).await {
        Ok(response) => Ok(response.categories),
        Err(error) => Err(error.reason),
    }
}

#[tokio::test]
async fn moderators_keep_the_list_of_categories() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    assert!(ctx.catalog().await.categories.is_empty());
    let empty_version = ctx.catalog().await.version;

    // Creating: at the end of the list, names tidied, no two alike.
    assert_eq!(shelf(&create(&ctx, &moderator, "どうぶつ").await.unwrap()), [("どうぶつ", 0)]);
    create(&ctx, &moderator, "  Ｍemes ").await.unwrap();
    let created = create(&ctx, &moderator, "たべもの/おかし").await.unwrap();
    assert_eq!(shelf(&created), [("どうぶつ", 0), ("Memes", 0), ("たべもの/おかし", 0)]);
    assert!(ctx.catalog().await.version > empty_version, "a new category is a new catalogue version");
    assert_eq!(create(&ctx, &moderator, "どうぶつ").await.unwrap_err(), "CATEGORY_EXISTS");
    assert_eq!(
        create(&ctx, &moderator, "MEMES").await.unwrap_err(),
        "CATEGORY_EXISTS",
        "case does not make a new category"
    );
    for bad in ["", "   ", "/cats", "cats/", &"c".repeat(33)] {
        assert_eq!(create(&ctx, &moderator, bad).await.unwrap_err(), "CATEGORY_NAME_INVALID", "{bad:?}");
    }

    // Emoji name their category; the list counts the approved ones.
    let animal =
        AddEmojiRequest { category: "どうぶつ".to_owned(), ..ctx.add_request(&moderator, &unique("neko")).await };
    let animal = ctx.add_with(&moderator, animal).await.emoji.id.clone();
    let meme = AddEmojiRequest { category: "memes".to_owned(), ..ctx.add_request(&moderator, &unique("doge")).await };
    let meme = ctx.add_with(&moderator, meme).await.emoji.id.clone();
    let loose = ctx.add(&moderator, &unique("loose")).await.emoji.id.clone();
    let waiting =
        SubmitEmojiRequest { category: "どうぶつ".to_owned(), ..ctx.submit_request(&author, &unique("inu")).await };
    let waiting: SubmitEmojiResponse = ctx.call("Submission", "SubmitEmoji", Some(&author), &waiting).await.unwrap();
    let waiting = waiting.emoji.emoji.id.clone();
    let listed: ListCategoriesResponse =
        ctx.call("", "ListCategories", None, &ListCategoriesRequest::default()).await.unwrap();
    assert_eq!(shelf(&listed.categories), [("どうぶつ", 1), ("Memes", 1), ("たべもの/おかし", 0)]);
    assert_eq!(listed.uncategorized_count, 1);
    assert_eq!(ctx.get(None, &meme).await.unwrap().emoji.category, "Memes", "stored under the category's own spelling");

    // Ordering: the whole list, each name once.
    let ordered = reorder(&ctx, &moderator, &["memes", "たべもの/おかし", "どうぶつ"]).await.unwrap();
    assert_eq!(shelf(&ordered), [("Memes", 1), ("たべもの/おかし", 0), ("どうぶつ", 1)]);
    assert_eq!(shelf(&ctx.catalog().await.categories), shelf(&ordered), "clients get the same order");
    for wrong in [
        &["Memes", "どうぶつ"][..],
        &["Memes", "どうぶつ", "どうぶつ"],
        &["Memes", "どうぶつ", "たべもの/おかし", "ほか"],
        &["Memes", "どうぶつ", "ほか"],
        &[],
    ] {
        assert_eq!(reorder(&ctx, &moderator, wrong).await.unwrap_err(), "ORDER_MISMATCH", "{wrong:?}");
    }

    // Renaming: the emoji follow, and clients learn of both.
    let version = ctx.catalog().await.version;
    let renamed = rename(&ctx, &moderator, "memes", "ミーム").await.unwrap();
    assert_eq!(shelf(&renamed), [("ミーム", 1), ("たべもの/おかし", 0), ("どうぶつ", 1)]);
    let changed = ctx.changes(version).await;
    let moved = changed.upserted.iter().find(|emoji| emoji.id == meme).expect("the renamed category's emoji changed");
    assert_eq!(moved.category, "ミーム");
    assert!(changed.upserted.iter().all(|emoji| emoji.id != animal), "other emoji did not");
    assert_eq!(shelf(&changed.categories), shelf(&renamed));
    let doc = ctx.search_changes(&meme).await.last().unwrap().clone().unwrap();
    assert_eq!(doc["text"], "ミーム", "the search document follows");
    assert_eq!(rename(&ctx, &moderator, "ミーム", "ミーム").await.unwrap().len(), 3, "renaming to itself is fine");
    assert_eq!(rename(&ctx, &moderator, "ミーム", "どうぶつ").await.unwrap_err(), "CATEGORY_EXISTS");
    assert_eq!(rename(&ctx, &moderator, "ほか", "べつ").await.unwrap_err(), "CATEGORY_NOT_FOUND");
    assert_eq!(rename(&ctx, &moderator, "ミーム", "/").await.unwrap_err(), "CATEGORY_NAME_INVALID");
    // Only the spelling changes: still the same category.
    rename(&ctx, &moderator, "たべもの/おかし", "たべもの / おかし").await.unwrap();

    // Deleting: the emoji move on, approved or not.
    assert_eq!(delete(&ctx, &moderator, "ほか", "").await.unwrap_err(), "CATEGORY_NOT_FOUND");
    assert_eq!(delete(&ctx, &moderator, "どうぶつ", "ほか").await.unwrap_err(), "CATEGORY_NOT_FOUND");
    assert_eq!(delete(&ctx, &moderator, "どうぶつ", "どうぶつ").await.unwrap_err(), "CATEGORY_NOT_FOUND");
    let version = ctx.catalog().await.version;
    let merged = delete(&ctx, &moderator, "どうぶつ", "ミーム").await.unwrap();
    assert_eq!(shelf(&merged), [("ミーム", 2), ("たべもの / おかし", 0)]);
    assert_eq!(ctx.get(None, &animal).await.unwrap().emoji.category, "ミーム");
    assert_eq!(ctx.get(Some(&author), &waiting).await.unwrap().emoji.category, "ミーム");
    let changed = ctx.changes(version).await;
    assert!(changed.upserted.iter().any(|emoji| emoji.id == animal && emoji.category == "ミーム"));

    let emptied = delete(&ctx, &moderator, "ミーム", "").await.unwrap();
    assert_eq!(shelf(&emptied), [("たべもの / おかし", 0)]);
    for id in [&animal, &meme, &loose] {
        assert_eq!(ctx.get(None, id).await.unwrap().emoji.category, "");
    }
    assert_eq!(ctx.get(Some(&author), &waiting).await.unwrap().emoji.category, "");
    let listed: ListCategoriesResponse =
        ctx.call("", "ListCategories", None, &ListCategoriesRequest::default()).await.unwrap();
    assert_eq!(listed.uncategorized_count, 3);
    // The same category added by many at once exists once.
    let results = join_all((0..8).map(|_| create(&ctx, &moderator, "どうじ"))).await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(results.iter().filter_map(|result| result.as_ref().err()).all(|reason| reason == "CATEGORY_EXISTS"));
    let listed: ListCategoriesResponse =
        ctx.call("", "ListCategories", None, &ListCategoriesRequest::default()).await.unwrap();
    assert_eq!(shelf(&listed.categories), [("たべもの / おかし", 0), ("どうじ", 0)]);

    // A deleted category cannot be named any more.
    let request =
        AddEmojiRequest { category: "ミーム".to_owned(), ..ctx.add_request(&moderator, &unique("late")).await };
    assert_eq!(ctx.reason("Admin", "AddEmoji", Some(&moderator), &request).await, "CATEGORY_NOT_FOUND");
}
