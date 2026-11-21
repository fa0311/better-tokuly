//! Activity kinds: validation, prefix matching, display names and the
//! mapping from an entry to the card type clients lay it out with.
//!
//! Kinds are dotted lowercase names chosen by the producing domain
//! (`social.bumped`, `note.posted`). A *kind prefix* matches whole segments:
//! `social` matches `social.bumped`, `social.bumped` matches only itself (and
//! anything below it), and `soc` matches nothing.
use tokuly_core::{outbound::Localized, prelude::*};
use tokuly_proto::{common::SubjectType, feed::ActivityCardType};

/// Recorded by this domain when an account is created.
pub const ACCOUNT_JOINED: &str = "account.joined";
/// Recorded by the social domain for each of the two people who bumped.
pub const BUMPED: &str = "social.bumped";
/// Recorded by the social domain when two people become friends.
pub const FRIEND_ADDED: &str = "social.friend_added";

const MAX_KIND_LEN: usize = 64;

/// Whether `kind` is a well-formed kind (or kind prefix): 1-64 characters of
/// `[a-z0-9_]` in dot-separated, non-empty segments.
pub fn is_valid(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= MAX_KIND_LEN
        && kind.split('.').all(|segment| !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
}

/// An optional kind-prefix filter from a request: empty means "everything".
pub fn parse_prefix(input: &str) -> AppResult<Option<String>> {
    let prefix = input.trim();
    if prefix.is_empty() {
        return Ok(None);
    }
    ensure!(is_valid(prefix), invalid, "INVALID_KIND", "invalid activity kind {prefix:?}");
    Ok(Some(prefix.to_owned()))
}

/// A kind (or kind prefix) that must be present.
pub fn parse_required(input: &str) -> AppResult<String> {
    match parse_prefix(input)? {
        Some(kind) => Ok(kind),
        None => bail!(invalid, "INVALID_KIND", "an activity kind is required"),
    }
}

/// Whether `kind` falls under `prefix` (see the module comment).
pub fn matches(kind: &str, prefix: &str) -> bool {
    kind == prefix || (kind.len() > prefix.len() && kind.starts_with(prefix) && kind.as_bytes()[prefix.len()] == b'.')
}

/// Display names of kinds and kind families, most specific entries included.
const LABELS: &[(&str, &str, &str)] = &[
    ("account", "アカウント", "Account"),
    (ACCOUNT_JOINED, "Tokulyに参加", "Joined Tokuly"),
    ("social", "友達", "Friends"),
    (BUMPED, "バンプ", "Bumps"),
    (FRIEND_ADDED, "友達追加", "New friends"),
    ("geo", "マップ", "Map"),
    ("geo.new_region", "新しい地域", "New regions"),
    ("geo.region", "地域", "Regions"),
    ("geo.new_place", "新しい場所", "New places"),
    ("geo.place", "場所", "Places"),
    ("geo.visit", "訪れた場所", "Visits"),
    ("geo.gathering", "集合", "Gatherings"),
    ("geo.trip", "おでかけ", "Trips"),
    ("place", "場所", "Places"),
    ("note", "ノート", "Notes"),
    ("story", "記憶", "Stories"),
    ("video", "動画", "Videos"),
    ("short", "ショート", "Shorts"),
    ("live", "ライブ配信", "Live streams"),
    ("clip", "クリップ", "Clips"),
    ("playlist", "再生リスト", "Playlists"),
    ("channel", "チャンネル", "Channels"),
    ("stamp", "スタンプ", "Stamps"),
    ("stamp.pack.published", "スタンプパック公開", "Stamp packs published"),
    ("chat", "チャット", "Chat"),
    ("wallet", "コイン", "Coins"),
];

/// Kinds and kind families every user can switch off for other people, even
/// before their log holds an entry of that kind.
pub const WELL_KNOWN: &[&str] = &[ACCOUNT_JOINED, BUMPED, FRIEND_ADDED, "geo", "note", "story", "video", "short", "live", "stamp.pack.published"];

/// The display name of a kind: its own label, else the label of the closest
/// family it belongs to, else the kind itself.
pub fn label(kind: &str) -> Localized {
    LABELS
        .iter()
        .filter(|(prefix, _, _)| matches(kind, prefix))
        .max_by_key(|(prefix, _, _)| prefix.len())
        .map(|(_, ja, en)| Localized::new(*ja, *en))
        .unwrap_or_else(|| Localized::same(kind))
}

use ActivityCardType as Card;

/// Kinds that decide the card type whatever the entry points at.
const CARD_KINDS: &[(&str, Card)] = &[
    (ACCOUNT_JOINED, Card::ACTIVITY_CARD_TYPE_JOINED),
    (BUMPED, Card::ACTIVITY_CARD_TYPE_BUMP),
    (FRIEND_ADDED, Card::ACTIVITY_CARD_TYPE_FRIENDSHIP),
    ("geo.new_region", Card::ACTIVITY_CARD_TYPE_REGION),
    ("geo.region", Card::ACTIVITY_CARD_TYPE_REGION),
    ("geo.new_place", Card::ACTIVITY_CARD_TYPE_PLACE),
    ("geo.place", Card::ACTIVITY_CARD_TYPE_PLACE),
];

/// What an entry points at decides the card type next.
const CARD_SUBJECTS: &[(SubjectType, Card)] = &[
    (SubjectType::SUBJECT_TYPE_NOTE, Card::ACTIVITY_CARD_TYPE_NOTE),
    (SubjectType::SUBJECT_TYPE_STORY, Card::ACTIVITY_CARD_TYPE_STORY),
    (SubjectType::SUBJECT_TYPE_VIDEO, Card::ACTIVITY_CARD_TYPE_VIDEO),
    (SubjectType::SUBJECT_TYPE_SHORT, Card::ACTIVITY_CARD_TYPE_SHORT),
    (SubjectType::SUBJECT_TYPE_LIVE_STREAM, Card::ACTIVITY_CARD_TYPE_LIVE),
    (SubjectType::SUBJECT_TYPE_CLIP, Card::ACTIVITY_CARD_TYPE_CLIP),
    (SubjectType::SUBJECT_TYPE_PLAYLIST, Card::ACTIVITY_CARD_TYPE_PLAYLIST),
    (SubjectType::SUBJECT_TYPE_STAMP_PACK, Card::ACTIVITY_CARD_TYPE_STAMP_PACK),
    (SubjectType::SUBJECT_TYPE_CHANNEL, Card::ACTIVITY_CARD_TYPE_CHANNEL),
    (SubjectType::SUBJECT_TYPE_PLACE, Card::ACTIVITY_CARD_TYPE_PLACE),
];

/// Kind families that give a card type to entries without a subject.
const CARD_FAMILIES: &[(&str, Card)] = &[
    ("note", Card::ACTIVITY_CARD_TYPE_NOTE),
    ("story", Card::ACTIVITY_CARD_TYPE_STORY),
    ("video", Card::ACTIVITY_CARD_TYPE_VIDEO),
    ("short", Card::ACTIVITY_CARD_TYPE_SHORT),
    ("live", Card::ACTIVITY_CARD_TYPE_LIVE),
    ("clip", Card::ACTIVITY_CARD_TYPE_CLIP),
    ("playlist", Card::ACTIVITY_CARD_TYPE_PLAYLIST),
    ("stamp.pack", Card::ACTIVITY_CARD_TYPE_STAMP_PACK),
    ("channel", Card::ACTIVITY_CARD_TYPE_CHANNEL),
    ("place", Card::ACTIVITY_CARD_TYPE_PLACE),
];

fn longest_match(table: &[(&'static str, Card)], kind: &str) -> Option<Card> {
    table.iter().filter(|(prefix, _)| matches(kind, prefix)).max_by_key(|(prefix, _)| prefix.len()).map(|(_, card)| *card)
}

/// The card type of an entry: what layout and link target a client picks.
/// `UNSPECIFIED` means "no special layout, show the summary".
pub fn card_type(kind: &str, subject_type: Option<i32>) -> Card {
    longest_match(CARD_KINDS, kind)
        .or_else(|| subject_type.and_then(|subject| CARD_SUBJECTS.iter().find(|(known, _)| *known as i32 == subject).map(|(_, card)| *card)))
        .or_else(|| longest_match(CARD_FAMILIES, kind))
        .unwrap_or(Card::ACTIVITY_CARD_TYPE_UNSPECIFIED)
}

/// Kind prefixes whose entries always have a card type. Together with
/// [`card_subject_types`] this is [`card_type`] expressed as data, for the
/// home feed's SQL filter.
pub fn card_kind_prefixes() -> Vec<String> {
    CARD_KINDS.iter().chain(CARD_FAMILIES).map(|(prefix, _)| (*prefix).to_owned()).collect()
}

/// Subject types whose entries always have a card type.
pub fn card_subject_types() -> Vec<i32> {
    CARD_SUBJECTS.iter().map(|(subject, _)| *subject as i32).collect()
}

/// Subject types that are a piece of content: two cards about the same one
/// are duplicates in the home feed. (Places and people are not: several
/// friends visiting the same place are several events.)
pub fn content_subject_types() -> Vec<i32> {
    use SubjectType as S;
    [
        S::SUBJECT_TYPE_NOTE,
        S::SUBJECT_TYPE_STORY,
        S::SUBJECT_TYPE_VIDEO,
        S::SUBJECT_TYPE_SHORT,
        S::SUBJECT_TYPE_LIVE_STREAM,
        S::SUBJECT_TYPE_CLIP,
        S::SUBJECT_TYPE_PLAYLIST,
        S::SUBJECT_TYPE_STAMP_PACK,
        S::SUBJECT_TYPE_CHANNEL,
    ]
    .iter()
    .map(|subject| *subject as i32)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_dotted_lowercase_segments() {
        for kind in ["a", "social.bumped", "stamp.pack.published", "geo.new_region", "x1.y_2"] {
            assert!(is_valid(kind), "{kind}");
        }
        for kind in ["", ".", "social.", ".bumped", "social..bumped", "Social.bumped", "social bumped", "ノート", "a-b", &"a".repeat(65)] {
            assert!(!is_valid(kind), "{kind}");
        }
    }

    #[test]
    fn prefixes_match_whole_segments_only() {
        assert!(matches("social.bumped", "social"));
        assert!(matches("social.bumped", "social.bumped"));
        assert!(matches("stamp.pack.published", "stamp.pack"));
        assert!(!matches("social.bumped", "soc"));
        assert!(!matches("social", "social.bumped"));
        assert!(!matches("socialite.joined", "social"));
    }

    #[test]
    fn prefix_filters_are_optional_but_must_be_well_formed() {
        assert_eq!(parse_prefix("").unwrap(), None);
        assert_eq!(parse_prefix("  geo ").unwrap().as_deref(), Some("geo"));
        assert_eq!(parse_prefix("Geo!").unwrap_err().reason, "INVALID_KIND");
        assert_eq!(parse_required(" ").unwrap_err().reason, "INVALID_KIND");
        assert_eq!(parse_required("note.posted").unwrap(), "note.posted");
    }

    #[test]
    fn labels_fall_back_from_kind_to_family_to_the_kind_itself() {
        assert_eq!(label("social.bumped").ja, "バンプ");
        assert_eq!(label("social.poked").en, "Friends");
        assert_eq!(label("note.posted").ja, "ノート");
        assert_eq!(label("stamp.pack.published").en, "Stamp packs published");
        assert_eq!(label("stamp.sent").ja, "スタンプ");
        assert_eq!(label("mystery.thing").ja, "mystery.thing");
    }

    #[test]
    fn card_types_come_from_the_kind_then_the_subject_then_the_family() {
        let user = Some(SubjectType::SUBJECT_TYPE_USER as i32);
        assert_eq!(card_type(BUMPED, user), Card::ACTIVITY_CARD_TYPE_BUMP);
        assert_eq!(card_type(FRIEND_ADDED, user), Card::ACTIVITY_CARD_TYPE_FRIENDSHIP);
        assert_eq!(card_type(ACCOUNT_JOINED, None), Card::ACTIVITY_CARD_TYPE_JOINED);
        assert_eq!(card_type("geo.new_region", None), Card::ACTIVITY_CARD_TYPE_REGION);
        assert_eq!(card_type("geo.new_place", Some(SubjectType::SUBJECT_TYPE_PLACE as i32)), Card::ACTIVITY_CARD_TYPE_PLACE);
        // The subject wins over the family: a renote is still a note card.
        assert_eq!(card_type("note.renoted", Some(SubjectType::SUBJECT_TYPE_NOTE as i32)), Card::ACTIVITY_CARD_TYPE_NOTE);
        assert_eq!(card_type("live.started", Some(SubjectType::SUBJECT_TYPE_LIVE_STREAM as i32)), Card::ACTIVITY_CARD_TYPE_LIVE);
        assert_eq!(card_type("video.published", None), Card::ACTIVITY_CARD_TYPE_VIDEO);
        assert_eq!(card_type("stamp.pack.published", Some(SubjectType::SUBJECT_TYPE_STAMP_PACK as i32)), Card::ACTIVITY_CARD_TYPE_STAMP_PACK);
        // Things without a layout of their own.
        assert_eq!(card_type("social.poked", user), Card::ACTIVITY_CARD_TYPE_UNSPECIFIED);
        assert_eq!(card_type("wallet.bonus", None), Card::ACTIVITY_CARD_TYPE_UNSPECIFIED);
    }

    /// The home feed selects cards in SQL with the two lists below; they must
    /// say "has a card type" exactly when [`card_type`] does.
    #[test]
    fn the_sql_side_lists_agree_with_card_type() {
        let prefixes = card_kind_prefixes();
        let subjects = card_subject_types();
        let listed = |kind: &str, subject: Option<i32>| prefixes.iter().any(|p| matches(kind, p)) || subject.is_some_and(|s| subjects.contains(&s));
        let subjects_to_try: Vec<Option<i32>> = std::iter::once(None).chain((0..=20).map(Some)).collect();
        for kind in [
            "account.joined",
            "social.bumped",
            "social.friend_added",
            "social.poked",
            "geo.new_region",
            "geo.region.explored",
            "geo.new_place",
            "geo.gathering",
            "note.posted",
            "note.renoted",
            "story.posted",
            "video.published",
            "short.posted",
            "live.started",
            "clip.created",
            "playlist.created",
            "stamp.pack.published",
            "stamp.sent",
            "channel.created",
            "wallet.bonus",
            "mystery",
        ] {
            for subject in &subjects_to_try {
                let typed = card_type(kind, *subject) != Card::ACTIVITY_CARD_TYPE_UNSPECIFIED;
                assert_eq!(typed, listed(kind, *subject), "{kind} with subject {subject:?}");
            }
        }
    }
}
