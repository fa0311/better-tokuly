//! Reaction decks: the emoji a user pinned, in their order. A deck holds
//! custom emoji (`:name:`) and Unicode emoji side by side. A user who never
//! saved one sees the server's default deck.
use tokuly_core::prelude::*;
use tokuly_proto::emoji::PickerItem;

use crate::{catalog::Snapshot, config::EmojiConfig, usage::Item, validate::DECK_MAX_ITEMS};

/// Reads the items of a deck a client wants to save. Duplicates are dropped
/// (the first occurrence keeps its place). Errors: `DECK_TOO_LONG`,
/// `DECK_ITEM_INVALID`.
pub fn parse(input: &[String]) -> AppResult<Vec<Item>> {
    ensure!(input.len() <= DECK_MAX_ITEMS, invalid, "DECK_TOO_LONG", "a deck holds at most {DECK_MAX_ITEMS} emoji");
    let mut items = Vec::with_capacity(input.len());
    for text in input {
        let Some(item) = Item::parse(text) else {
            bail!(invalid, "DECK_ITEM_INVALID", "a deck item is :name: or one emoji, got {text:?}");
        };
        if !items.contains(&item) {
            items.push(item);
        }
    }
    Ok(items)
}

/// The default deck in stored form; configured items that are no emoji are
/// left out.
pub fn default_items(cfg: &EmojiConfig) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for item in cfg.default_deck.iter().filter_map(|text| Item::parse(text)) {
        let text = item.text();
        if items.len() < DECK_MAX_ITEMS && !items.contains(&text) {
            items.push(text);
        }
    }
    items
}

/// The deck a user saved, in stored form; `None` when they never saved one.
pub async fn load(core: &Core, user_id: Uuid) -> AppResult<Option<Vec<String>>> {
    Ok(sqlx::query_scalar("SELECT items FROM emoji_decks WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&core.db)
        .await?)
}

pub async fn save(core: &Core, user_id: Uuid, items: &[String]) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO emoji_decks (user_id, items) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET items = EXCLUDED.items",
    )
    .bind(user_id)
    .bind(items)
    .execute(&core.db)
    .await?;
    Ok(())
}

/// Forgets the user's deck: they are back on the default one.
pub async fn reset(core: &Core, user_id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM emoji_decks WHERE user_id = $1").bind(user_id).execute(&core.db).await?;
    Ok(())
}

/// Stored items as clients get them: custom emoji come with their catalogue
/// entry, or without one when they are not in the catalogue (any more).
pub fn picker_items(snapshot: &Snapshot, stored: &[String]) -> Vec<PickerItem> {
    stored
        .iter()
        .filter_map(|text| Item::parse(text))
        .map(|item| {
            let custom = match &item {
                Item::Custom(name) => snapshot.get(name).cloned(),
                Item::Unicode(_) => None,
            };
            PickerItem { emoji: item.text(), custom: custom.into(), ..Default::default() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn decks_hold_custom_and_unicode_emoji_once_each() {
        let items = parse(&strings(&[":BlobCat:", "👍", "blobcat", "🎉"])).unwrap();
        let texts: Vec<String> = items.iter().map(Item::text).collect();
        assert_eq!(texts, [":blobcat:", "👍", "🎉"]);
        assert!(parse(&[]).unwrap().is_empty());
    }

    #[test]
    fn decks_refuse_what_is_no_emoji_and_overlong_lists() {
        assert_eq!(parse(&strings(&["hello world"])).unwrap_err().reason, "DECK_ITEM_INVALID");
        assert_eq!(parse(&strings(&["👍", ""])).unwrap_err().reason, "DECK_ITEM_INVALID");
        let long: Vec<String> = (0..=DECK_MAX_ITEMS).map(|i| format!(":emoji_{i}:")).collect();
        assert_eq!(parse(&long).unwrap_err().reason, "DECK_TOO_LONG");
        assert_eq!(parse(&long[..DECK_MAX_ITEMS]).unwrap().len(), DECK_MAX_ITEMS);
    }

    #[test]
    fn the_default_deck_is_what_the_configuration_names() {
        let default = default_items(&EmojiConfig::default());
        assert_eq!(default.len(), 10);
        assert_eq!(default[0], "👍");
        let custom =
            EmojiConfig { default_deck: strings(&[":tokuly:", "not an emoji", "👍", "👍"]), ..Default::default() };
        assert_eq!(default_items(&custom), [":tokuly:", "👍"]);
    }
}
