//! Custom emoji as reactions: checking one when it is added, and dressing
//! reactions with what a client needs to draw them.
//!
//! A custom emoji reaction is stored as its shortcode (`:neko:`), the way
//! it is written in text. Its picture is never stored here: it is looked up
//! in the emoji domain's cached catalogue whenever reactions are returned,
//! all names of a response at once ([`Pictures::load`], one Valkey read). A
//! name the catalogue no longer has — the emoji was disabled, deleted or
//! renamed — comes back as unavailable, and its reactions keep counting.
//! Like Misskey, reactions follow the name: a renamed emoji's old reactions
//! show the old shortcode.
use std::collections::{BTreeSet, HashMap};

use tokuly_core::prelude::*;
use tokuly_proto::{
    common::Image,
    emoji::CustomEmoji,
    engagement::{Reaction, ReactionEmoji, ReactionSummary},
};

use super::{Stored, rules::Rules};

/// The smallest still that serves as the reduced-motion picture: emoji are
/// drawn at text size, twice that for sharp screens.
const STILL_MIN_SIZE: i32 = 64;

/// The name inside a custom emoji reaction (`:neko:` → `neko`); `None` for
/// a Unicode emoji.
pub(crate) fn custom_name(emoji: &str) -> Option<&str> {
    emoji.strip_prefix(':').and_then(|rest| rest.strip_suffix(':')).filter(|name| !name.is_empty())
}

/// Checks an emoji someone wants to add against the catalogue and the
/// subject's rules. Unicode emoji pass. Reads Valkey only (and the
/// database through the pool when the cached catalogue is stale), so it
/// runs before the reaction's transaction opens.
pub(crate) async fn check(core: &Core, reaction: &Stored, rules: &Rules) -> AppResult<()> {
    let Some(name) = custom_name(&reaction.emoji) else { return Ok(()) };
    ensure!(rules.custom_emoji, precondition, "CUSTOM_EMOJI_NOT_ALLOWED", "custom emoji are turned off here");
    let Some(emoji) = tokuly_emoji::catalog::lookup(core, &[name.to_owned()]).await?.remove(name) else {
        return Err(AppError::precondition("EMOJI_UNAVAILABLE", "there is no such emoji, or it was taken down"));
    };
    ensure!(
        !emoji.sensitive || rules.sensitive_emoji,
        precondition,
        "SENSITIVE_EMOJI_NOT_ALLOWED",
        "this emoji is marked sensitive and is not allowed here"
    );
    Ok(())
}

/// What a client needs to draw an approved emoji.
fn picture(emoji: &CustomEmoji) -> ReactionEmoji {
    let image = emoji.image.as_option().cloned().unwrap_or_default();
    ReactionEmoji {
        name: emoji.name.clone(),
        static_url: still_url(&image, emoji.animated),
        image: image.into(),
        animated: emoji.animated,
        sensitive: emoji.sensitive,
        available: true,
        ..Default::default()
    }
}

/// A still rendition: the smallest still of at least [`STILL_MIN_SIZE`],
/// else the largest; the picture itself for a still emoji without
/// renditions; empty for an animated one without any.
fn still_url(image: &Image, animated: bool) -> String {
    let mut stills: Vec<_> = image.variants.iter().filter(|variant| !variant.url.is_empty()).collect();
    stills.sort_by_key(|variant| variant.size);
    match stills.iter().find(|variant| variant.size >= STILL_MIN_SIZE).or(stills.last()) {
        Some(variant) => variant.url.clone(),
        None if !animated => image.url.clone(),
        None => String::new(),
    }
}

/// The pictures of the custom emoji in a response, by reaction text
/// (`:neko:`).
#[derive(Debug, Default)]
pub(crate) struct Pictures(HashMap<String, ReactionEmoji>);

impl Pictures {
    /// Looks up every custom emoji among `emoji` (reaction texts; Unicode
    /// emoji and empty strings are skipped) in one catalogue read.
    pub(crate) async fn load<'a>(core: &Core, emoji: impl IntoIterator<Item = &'a str>) -> AppResult<Self> {
        let names: BTreeSet<&str> = emoji.into_iter().filter_map(custom_name).collect();
        if names.is_empty() {
            return Ok(Self::default());
        }
        let names: Vec<String> = names.into_iter().map(str::to_owned).collect();
        let mut found = tokuly_emoji::catalog::lookup(core, &names).await?;
        Ok(Self(
            names
                .into_iter()
                .map(|name| {
                    let shown = match found.remove(&name) {
                        Some(emoji) => picture(&emoji),
                        None => ReactionEmoji { name: name.clone(), available: false, ..Default::default() },
                    };
                    (format!(":{name}:"), shown)
                })
                .collect(),
        ))
    }

    /// The pictures for every emoji named in `summaries`.
    pub(crate) async fn for_summaries(core: &Core, summaries: &[&ReactionSummary]) -> AppResult<Self> {
        Self::load(core, summaries.iter().flat_map(|summary| emoji_of(summary))).await
    }

    /// The picture of a custom emoji reaction text, if it is one.
    pub(crate) fn get(&self, emoji: &str) -> Option<&ReactionEmoji> {
        self.0.get(emoji)
    }

    /// `reaction` as a client sees it: with its picture when it is a custom emoji.
    pub(crate) fn reaction(&self, reaction: &Stored) -> Reaction {
        let mut shown = reaction.to_proto();
        self.dress(&mut shown);
        shown
    }

    pub(crate) fn dress(&self, reaction: &mut Reaction) {
        if let Some(picture) = self.get(&reaction.emoji) {
            reaction.custom = picture.clone().into();
        }
    }

    pub(crate) fn dress_summary(&self, summary: &mut ReactionSummary) {
        for count in &mut summary.emoji_counts {
            if let Some(picture) = self.get(&count.emoji) {
                count.custom = picture.clone().into();
            }
        }
        if let Some(reaction) = summary.viewer_reaction.as_option_mut() {
            self.dress(reaction);
        }
        for reaction in &mut summary.viewer_reactions {
            self.dress(reaction);
        }
    }
}

/// Every emoji text a summary names.
fn emoji_of(summary: &ReactionSummary) -> impl Iterator<Item = &str> {
    summary
        .emoji_counts
        .iter()
        .map(|count| count.emoji.as_str())
        .chain(summary.viewer_reaction.as_option().map(|reaction| reaction.emoji.as_str()))
        .chain(summary.viewer_reactions.iter().map(|reaction| reaction.emoji.as_str()))
}

#[cfg(test)]
mod tests {
    use tokuly_proto::common::ImageVariant;

    use super::*;

    #[test]
    fn custom_names_are_the_inside_of_shortcodes() {
        assert_eq!(custom_name(":neko:"), Some("neko"));
        assert_eq!(custom_name("🎉"), None);
        assert_eq!(custom_name("::"), None);
        assert_eq!(custom_name(""), None);
    }

    fn variant(size: i32) -> ImageVariant {
        ImageVariant { size, url: format!("https://media.test/{size}.webp"), ..Default::default() }
    }

    #[test]
    fn the_still_is_the_smallest_sharp_enough_rendition() {
        let image = Image {
            url: "https://media.test/anim.webp".into(),
            variants: vec![variant(256), variant(32), variant(96)],
            ..Default::default()
        };
        assert_eq!(still_url(&image, true), "https://media.test/96.webp");
        let small = Image { variants: vec![variant(16), variant(32)], ..Default::default() };
        assert_eq!(still_url(&small, true), "https://media.test/32.webp");
        let bare = Image { url: "https://media.test/still.png".into(), ..Default::default() };
        assert_eq!(still_url(&bare, false), "https://media.test/still.png");
        assert_eq!(still_url(&bare, true), "");
    }

    #[test]
    fn approved_emoji_are_drawn_and_missing_ones_fall_back() {
        let emoji = CustomEmoji {
            name: "neko".into(),
            animated: true,
            sensitive: true,
            image: Image {
                url: "https://media.test/neko.webp".into(),
                variants: vec![variant(128)],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        let shown = picture(&emoji);
        assert!(shown.available && shown.animated && shown.sensitive);
        assert_eq!(shown.static_url, "https://media.test/128.webp");
        assert_eq!(shown.image.as_option().unwrap().url, "https://media.test/neko.webp");

        let mut pictures = Pictures::default();
        pictures.0.insert(":neko:".into(), shown);
        pictures.0.insert(":gone:".into(), ReactionEmoji { name: "gone".into(), ..Default::default() });
        let mut summary = ReactionSummary {
            emoji_counts: vec![
                tokuly_proto::engagement::EmojiCount { emoji: ":gone:".into(), count: 2, ..Default::default() },
                tokuly_proto::engagement::EmojiCount { emoji: "🎉".into(), count: 1, ..Default::default() },
            ],
            viewer_reactions: vec![Stored { kind: 3, emoji: ":neko:".into() }.to_proto()],
            ..Default::default()
        };
        pictures.dress_summary(&mut summary);
        assert!(!summary.emoji_counts[0].custom.as_option().unwrap().available);
        assert!(summary.emoji_counts[1].custom.as_option().is_none());
        assert!(summary.viewer_reactions[0].custom.as_option().unwrap().available);
    }
}
