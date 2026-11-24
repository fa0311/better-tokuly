//! The プロフ帳: the parts of a profile that make it somebody's own.
//!
//! MBTI, blood type, zodiac (derived from the birthday), pronouns, 推し,
//! favourite things as tags, links (verified by a link back, see
//! [`crate::link_check`]), a longer bio, a theme, a pinned song and
//! question-and-answer cards from a curated bank of prompts ([`PROMPTS`]) or
//! free ones.
//!
//! **Audiences.** Every field but the theme has an audience of its own
//! (`ProfileFieldVisibility`), every card too: public, friends, or only the
//! owner. [`Extras::view`] is the single place that applies them; the
//! profile asks it with what the viewer is to the owner ([`Viewer`]), after
//! the profile extensions said whether the viewer may see the profile at
//! all — a pair in a block, either way, sees no profile and so nothing of
//! this. Moderators get everything ([`Viewer::Moderator`]), so a reported
//! profile can be judged on what its owner wrote whatever its audience.
//!
//! **Storage.** Single values in `profile_extras` (one row per account that
//! filled anything), lists in tables of their own with a `position`. An
//! update replaces whole groups under a lock on the account's row, so two
//! saves at once apply one after the other instead of interleaving.
//!
//! **Search.** What is public and worth finding someone by — favourite
//! things, 推し, the long bio — goes into the account's search document
//! through [`search_terms`].
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Datelike as _, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use tokuly_core::{events, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{
    common::{AudienceRef, LocalizedText, Visibility},
    identity::*,
    media::AssetPurpose,
};
use unicode_normalization::UnicodeNormalization as _;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{Identity, maintenance::UserRef};

pub const MAX_OSHI: usize = 10;
pub const MAX_TAGS: usize = 30;
pub const MAX_LINKS: usize = 8;
pub const MAX_CARDS: usize = 30;
const LONG_BIO_MAX: usize = 2000;

/// Job that checks an account's links for a link back.
pub const LINK_CHECK_JOB: &str = "identity.profile.links";

/// How close a viewer is to an owner, as the social graph says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tier {
    Friend,
    /// Follows the owner (and is not a friend).
    Follower,
    /// Anyone else, signed in or not.
    #[default]
    Other,
}

/// What a viewer is to an owner for the audiences of the プロフ帳: the
/// tier, whether the owner's account is locked, whether the viewer is in
/// the owner's close friends, and which of the owner's circles (among those
/// asked about) the viewer is in. Filled by the profile extensions (the
/// social domain's audience API, wired by the server).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudienceTie {
    pub tier: Tier,
    pub owner_locked: bool,
    pub close_friend: bool,
    pub circles: Vec<Uuid>,
}

/// What the viewer of a profile is to its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Viewer {
    Owner,
    /// A moderator reading a reported account (see
    /// [`crate::Identity::extras_for_moderation`]): everything that reaches
    /// anybody, with its audience; not what the owner keeps to themself
    /// ("only me" fields, hidden cards).
    Moderator,
    Audience(AudienceTie),
}

impl Viewer {
    /// A stranger (or someone signed out) looking at an unlocked account.
    pub fn stranger() -> Self {
        Self::Audience(AudienceTie::default())
    }
}

/// Whether something shared with `visibility` (and `circle` for
/// `VISIBILITY_CIRCLE`) reaches `viewer`, with the meaning of the social
/// domain's audiences. An allowlist: a value not named here (unset, or one
/// added later) reaches nobody but the owner.
pub fn reaches(visibility: i32, circle: Option<Uuid>, viewer: &Viewer) -> bool {
    use buffa::Enumeration as _;
    let tie = match viewer {
        Viewer::Owner => return true,
        // A moderator sees what reached somebody: every audience but "only me".
        Viewer::Moderator => {
            return Visibility::from_i32(visibility).is_some_and(|v| {
                matches!(
                    v,
                    Visibility::VISIBILITY_PUBLIC
                        | Visibility::VISIBILITY_FOLLOWERS
                        | Visibility::VISIBILITY_FRIENDS
                        | Visibility::VISIBILITY_CLOSE_FRIENDS
                        | Visibility::VISIBILITY_CIRCLE
                )
            });
        }
        Viewer::Audience(tie) => tie,
    };
    let tied = matches!(tie.tier, Tier::Friend | Tier::Follower);
    let friend = tie.tier == Tier::Friend;
    match Visibility::from_i32(visibility) {
        Some(Visibility::VISIBILITY_PUBLIC) => !tie.owner_locked || tied,
        Some(Visibility::VISIBILITY_FOLLOWERS) => tied,
        Some(Visibility::VISIBILITY_FRIENDS) => friend,
        Some(Visibility::VISIBILITY_CLOSE_FRIENDS) => tied && tie.close_friend,
        Some(Visibility::VISIBILITY_CIRCLE) => tied && circle.is_some_and(|c| tie.circles.contains(&c)),
        _ => false,
    }
}

/// `identity.profile.updated`: what changed, so search and feeds can react
/// to the parts they show.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileUpdated {
    pub user_id: Uuid,
    /// Groups that changed: `mbti`, `blood_type`, `pronouns`, `long_bio`,
    /// `theme`, `visibility`, `oshi`, `tags`, `links`, `qa_cards`, `pinned_song`.
    pub changed: Vec<String>,
}

// ───────────────────────────── prompts ─────────────────────────────

/// A question of the curated bank: `(id, group, emoji, ja, en)`.
pub struct Prompt {
    pub id: &'static str,
    pub group: &'static str,
    pub emoji: &'static str,
    pub ja: &'static str,
    pub en: &'static str,
}

const fn prompt(
    id: &'static str,
    group: &'static str,
    emoji: &'static str,
    ja: &'static str,
    en: &'static str,
) -> Prompt {
    Prompt { id, group, emoji, ja, en }
}

/// The curated questions of the プロフ帳. IDs are stored with the answers:
/// never reuse or rename one; retire a prompt by leaving it in place.
pub const PROMPTS: &[Prompt] = &[
    prompt("fav_subject", "school", "📚", "好きな教科", "Favourite subject"),
    prompt("club", "school", "🏸", "部活・サークル", "Club or team"),
    prompt("school_event", "school", "🎪", "好きな学校行事", "Favourite school event"),
    prompt("lunch", "school", "🍱", "お昼ごはんの定番", "Go-to lunch"),
    prompt("hooked_on", "likes", "✨", "最近ハマってること", "What I'm into lately"),
    prompt("fav_food", "likes", "🍜", "好きな食べ物", "Favourite food"),
    prompt("fav_song", "likes", "🎧", "今いちばん聴いてる曲", "Song on repeat"),
    prompt("fav_place", "likes", "📍", "好きな場所", "Favourite place"),
    prompt("convenience_store", "likes", "🏪", "コンビニで絶対買うもの", "Always grab at the konbini"),
    prompt("karaoke", "likes", "🎤", "カラオケの十八番", "Karaoke go-to"),
    prompt("weekend", "me", "🛋️", "休日の過ごし方", "How I spend weekends"),
    prompt("nickname", "me", "🏷️", "呼ばれたいあだ名", "Call me"),
    prompt("treasure", "me", "💎", "宝物", "My treasure"),
    prompt("dream", "me", "🌈", "将来の夢", "Dream for the future"),
    prompt("personality", "me", "🫶", "よく言われる性格", "People say I'm…"),
    prompt("morning", "me", "⏰", "朝型？夜型？", "Morning or night person?"),
    prompt("desert_island", "if", "🏝️", "無人島に1つ持っていくなら", "One thing to take to a desert island"),
    prompt("superpower", "if", "🦸", "超能力をひとつもらえるなら", "If I could have one superpower"),
    prompt("million_yen", "if", "💴", "100万円あったら", "With a million yen I'd…"),
    prompt("time_travel", "if", "⏳", "タイムスリップするならいつ？", "Time travel to when?"),
    prompt("reborn", "if", "🐾", "生まれ変わったら何になりたい？", "In my next life I'll be…"),
];

pub fn prompt_by_id(id: &str) -> Option<&'static Prompt> {
    PROMPTS.iter().find(|p| p.id == id)
}

pub fn prompt_messages() -> Vec<ProfilePrompt> {
    PROMPTS
        .iter()
        .map(|p| ProfilePrompt {
            id: p.id.to_owned(),
            question: LocalizedText { ja: p.ja.to_owned(), en: p.en.to_owned(), ..Default::default() }.into(),
            group: p.group.to_owned(),
            emoji: p.emoji.to_owned(),
            ..Default::default()
        })
        .collect()
}

// ───────────────────────────── zodiac ─────────────────────────────

/// The western zodiac sign of a birthday.
pub fn zodiac_of(date: NaiveDate) -> Zodiac {
    use Zodiac::*;
    // (month, first day of the next sign in that month, sign until then)
    const SIGNS: [(u32, u32, Zodiac, Zodiac); 12] = [
        (1, 20, ZODIAC_CAPRICORN, ZODIAC_AQUARIUS),
        (2, 19, ZODIAC_AQUARIUS, ZODIAC_PISCES),
        (3, 21, ZODIAC_PISCES, ZODIAC_ARIES),
        (4, 20, ZODIAC_ARIES, ZODIAC_TAURUS),
        (5, 21, ZODIAC_TAURUS, ZODIAC_GEMINI),
        (6, 22, ZODIAC_GEMINI, ZODIAC_CANCER),
        (7, 23, ZODIAC_CANCER, ZODIAC_LEO),
        (8, 23, ZODIAC_LEO, ZODIAC_VIRGO),
        (9, 23, ZODIAC_VIRGO, ZODIAC_LIBRA),
        (10, 24, ZODIAC_LIBRA, ZODIAC_SCORPIO),
        (11, 23, ZODIAC_SCORPIO, ZODIAC_SAGITTARIUS),
        (12, 22, ZODIAC_SAGITTARIUS, ZODIAC_CAPRICORN),
    ];
    let (_, cusp, before, after) = SIGNS[date.month0() as usize];
    if date.day() < cusp { before } else { after }
}

// ───────────────────────────── validation ─────────────────────────────

fn graphemes(text: &str) -> usize {
    text.graphemes(true).count()
}

fn single_line(text: &str, max: usize, reason: &'static str, what: &str) -> AppResult<String> {
    let text = text.trim();
    ensure!(graphemes(text) <= max, invalid, reason, "{what} must be at most {max} characters");
    ensure!(!text.chars().any(char::is_control), invalid, reason, "{what} contains control characters");
    Ok(text.to_owned())
}

/// `ENFP`, `infj-t` → `INFJ-T`; empty clears.
pub fn validate_mbti(input: &str) -> AppResult<String> {
    let mbti = input.trim().to_ascii_uppercase();
    if mbti.is_empty() {
        return Ok(mbti);
    }
    let (letters, variant) = match mbti.split_once('-') {
        Some((letters, variant)) => (letters, Some(variant)),
        None => (mbti.as_str(), None),
    };
    let b = letters.as_bytes();
    let ok = b.len() == 4
        && matches!(b[0], b'E' | b'I')
        && matches!(b[1], b'S' | b'N')
        && matches!(b[2], b'T' | b'F')
        && matches!(b[3], b'J' | b'P')
        && variant.is_none_or(|v| v == "A" || v == "T");
    ensure!(ok, invalid, "MBTI_INVALID", "MBTI is four letters like ENFP, optionally -A or -T");
    Ok(mbti)
}

pub fn validate_pronouns(input: &str) -> AppResult<String> {
    single_line(input, 24, "PRONOUNS_INVALID", "pronouns")
}

/// Up to 2000 characters; line breaks and tabs are kept, other control
/// characters are not allowed, and runs of blank lines are folded.
pub fn validate_long_bio(input: &str) -> AppResult<String> {
    let text = input.trim().replace("\r\n", "\n");
    ensure!(
        graphemes(&text) <= LONG_BIO_MAX,
        invalid,
        "LONG_BIO_TOO_LONG",
        "the long bio must be at most {LONG_BIO_MAX} characters"
    );
    ensure!(
        !text.chars().any(|c| c.is_control() && c != '\n' && c != '\t'),
        invalid,
        "LONG_BIO_TOO_LONG",
        "the long bio contains control characters"
    );
    let mut folded = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.split('\n') {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 2 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        if !folded.is_empty() {
            folded.push('\n');
        }
        folded.push_str(line.trim_end());
    }
    Ok(folded)
}

pub fn validate_theme(theme: &ProfileTheme) -> AppResult<ProfileTheme> {
    let color = theme.color.trim().to_ascii_lowercase();
    ensure!(
        color.is_empty()
            || (color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit())),
        invalid,
        "THEME_INVALID",
        "the theme colour must be #rrggbb"
    );
    ensure!(theme.style.as_known().is_some(), invalid, "THEME_INVALID", "unknown theme style");
    Ok(ProfileTheme { color, style: theme.style, ..Default::default() })
}

/// The audiences offered for profile fields: public, followers, friends,
/// close friends, only me.
pub fn check_visibility(value: &buffa::EnumValue<Visibility>, field: &str) -> AppResult<i32> {
    use Visibility::*;
    match value.as_known() {
        Some(
            v @ (VISIBILITY_PUBLIC
            | VISIBILITY_FOLLOWERS
            | VISIBILITY_FRIENDS
            | VISIBILITY_CLOSE_FRIENDS
            | VISIBILITY_PRIVATE),
        ) => Ok(v as i32),
        _ => Err(AppError::invalid(
            "VISIBILITY_INVALID",
            format!("{field} must be public, followers, friends, close friends or private"),
        )),
    }
}

/// A card's audience: one of the field audiences, or one of the owner's circles.
fn check_card_audience(value: &buffa::EnumValue<Visibility>) -> AppResult<i32> {
    match value.as_known() {
        Some(Visibility::VISIBILITY_CIRCLE) => Ok(Visibility::VISIBILITY_CIRCLE as i32),
        _ => check_visibility(value, "audience"),
    }
}

/// An `http(s)` URL as stored: parsed, without credentials, at most 500 characters.
pub fn validate_link_url(input: &str, reason: &'static str, https_only: bool) -> AppResult<String> {
    let raw = input.trim();
    let url = url::Url::parse(raw).map_err(|_| AppError::invalid(reason, "enter a full web address (https://…)"))?;
    let scheme_ok = if https_only { url.scheme() == "https" } else { matches!(url.scheme(), "http" | "https") };
    ensure!(
        scheme_ok
            && url.host_str().is_some_and(|h| h.contains('.'))
            && url.username().is_empty()
            && url.password().is_none(),
        invalid,
        reason,
        "enter a full web address (https://…)"
    );
    let url = url.to_string();
    ensure!(url.chars().count() <= 500, invalid, reason, "the address must be at most 500 characters");
    Ok(url)
}

/// A tag name folded for matching and counting: NFKC (full-width letters
/// and half-width kana become their usual forms), lowercase, inner spaces
/// collapsed.
pub fn tag_key(name: &str) -> String {
    let normalized: String = name.nfkc().collect::<String>().to_lowercase();
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_tag(tag: &ProfileTag) -> AppResult<(i32, String, String)> {
    let category = match tag.category.as_known() {
        Some(ProfileTagCategory::PROFILE_TAG_CATEGORY_UNSPECIFIED) | None => {
            bail!(invalid, "TAG_INVALID", "every favourite thing needs a category")
        }
        Some(category) => category as i32,
    };
    let name: String = tag.name.split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!((1..=24).contains(&graphemes(&name)), invalid, "TAG_INVALID", "a favourite thing is 1-24 characters");
    ensure!(
        !name.chars().any(char::is_control) && !name.contains(['#', ',']),
        invalid,
        "TAG_INVALID",
        "a favourite thing cannot contain # or ,"
    );
    let key = tag_key(&name);
    Ok((category, name, key))
}

// ───────────────────────────── rows ─────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ExtrasRow {
    mbti: String,
    blood_type: i32,
    pronouns: String,
    long_bio: String,
    theme: serde_json::Value,
    pinned_song: Option<serde_json::Value>,
    mbti_visibility: i32,
    blood_type_visibility: i32,
    pronouns_visibility: i32,
    zodiac_visibility: i32,
    oshi_visibility: i32,
    tags_visibility: i32,
    links_visibility: i32,
    long_bio_visibility: i32,
    pinned_song_visibility: i32,
}

impl Default for ExtrasRow {
    /// What an account that never filled the プロフ帳 has: nothing, with the
    /// column defaults as audiences (the zodiac starts private).
    fn default() -> Self {
        let public = Visibility::VISIBILITY_PUBLIC as i32;
        Self {
            mbti: String::new(),
            blood_type: 0,
            pronouns: String::new(),
            long_bio: String::new(),
            theme: serde_json::json!({}),
            pinned_song: None,
            mbti_visibility: public,
            blood_type_visibility: public,
            pronouns_visibility: public,
            zodiac_visibility: Visibility::VISIBILITY_PRIVATE as i32,
            oshi_visibility: public,
            tags_visibility: public,
            links_visibility: public,
            long_bio_visibility: public,
            pinned_song_visibility: public,
        }
    }
}

const EXTRAS_COLUMNS: &str = "mbti, blood_type, pronouns, long_bio, theme, pinned_song, mbti_visibility, blood_type_visibility, \
    pronouns_visibility, zodiac_visibility, oshi_visibility, tags_visibility, links_visibility, long_bio_visibility, \
    pinned_song_visibility";

#[derive(Debug, Clone, sqlx::FromRow)]
struct OshiRow {
    id: Uuid,
    name: String,
    image: Option<serde_json::Value>,
    url: String,
    emoji: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct TagRow {
    category: i32,
    name: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct LinkRow {
    id: Uuid,
    title: String,
    url: String,
    verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct CardRow {
    id: Uuid,
    prompt_id: String,
    question: String,
    answer: String,
    visibility: i32,
    circle_id: Option<Uuid>,
    hidden: bool,
}

/// Name under which an oshi's image holds its media asset.
pub(crate) fn oshi_asset_holder(oshi_id: Uuid) -> String {
    format!("identity:oshi:{oshi_id}")
}

/// Everything of one account's プロフ帳, as stored.
#[derive(Debug, Clone, Default)]
pub struct Extras {
    row: ExtrasRow,
    oshi: Vec<OshiRow>,
    tags: Vec<TagRow>,
    links: Vec<LinkRow>,
    cards: Vec<CardRow>,
    birthdate: Option<NaiveDate>,
}

impl Extras {
    /// Loads the プロフ帳 of `user_id`; `birthdate` is the account's (for the zodiac).
    pub async fn load(core: &Core, user_id: Uuid, birthdate: Option<NaiveDate>) -> AppResult<Self> {
        let row: Option<ExtrasRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {EXTRAS_COLUMNS} FROM profile_extras WHERE user_id = $1"
        )))
        .bind(user_id)
        .fetch_optional(&core.db)
        .await?;
        let Some(row) = row else {
            return Ok(Self { birthdate, ..Default::default() });
        };
        let oshi = sqlx::query_as(
            "SELECT id, name, image, url, emoji FROM profile_oshi WHERE user_id = $1 ORDER BY position, id",
        )
        .bind(user_id)
        .fetch_all(&core.db)
        .await?;
        let tags =
            sqlx::query_as("SELECT category, name FROM profile_tags WHERE user_id = $1 ORDER BY position, name_key")
                .bind(user_id)
                .fetch_all(&core.db)
                .await?;
        let links = sqlx::query_as(
            "SELECT id, title, url, verified_at FROM profile_links WHERE user_id = $1 ORDER BY position, id",
        )
        .bind(user_id)
        .fetch_all(&core.db)
        .await?;
        let cards = sqlx::query_as(
            "SELECT id, prompt_id, question, answer, visibility, circle_id, hidden FROM profile_qa_cards WHERE user_id = $1 ORDER BY position, id",
        )
        .bind(user_id)
        .fetch_all(&core.db)
        .await?;
        Ok(Self { row, oshi, tags, links, cards, birthdate })
    }

    /// The プロフ帳 as `viewer` sees it. The owner and moderators get every
    /// field with its audience and the hidden cards; everyone else only what
    /// reaches them, without the audiences.
    /// The circles the cards are shared with, to ask the social graph about.
    pub fn card_circles(&self) -> Vec<Uuid> {
        let mut circles: Vec<Uuid> = self.cards.iter().filter_map(|c| c.circle_id).collect();
        circles.sort_unstable();
        circles.dedup();
        circles
    }

    pub fn view(&self, viewer: &Viewer) -> ProfileExtras {
        let r = &self.row;
        let full = matches!(viewer, Viewer::Owner | Viewer::Moderator);
        let shows = |visibility: i32| reaches(visibility, None, viewer);
        let mut extras = ProfileExtras {
            theme: serde_json::from_value::<ProfileTheme>(r.theme.clone()).unwrap_or_default().into(),
            ..Default::default()
        };
        if shows(r.mbti_visibility) {
            extras.mbti = r.mbti.clone();
        }
        if shows(r.blood_type_visibility) {
            extras.blood_type = r.blood_type.into();
        }
        if shows(r.pronouns_visibility) {
            extras.pronouns = r.pronouns.clone();
        }
        if shows(r.zodiac_visibility)
            && let Some(birthdate) = self.birthdate
        {
            extras.zodiac = zodiac_of(birthdate).into();
        }
        if shows(r.oshi_visibility) {
            extras.oshi = self
                .oshi
                .iter()
                .map(|o| Oshi {
                    id: o.id.to_string(),
                    name: o.name.clone(),
                    image: assets::image_from_json(o.image.as_ref()).into(),
                    url: o.url.clone(),
                    emoji: o.emoji.clone(),
                    ..Default::default()
                })
                .collect();
        }
        if shows(r.tags_visibility) {
            extras.tags = self
                .tags
                .iter()
                .map(|t| ProfileTag { category: t.category.into(), name: t.name.clone(), ..Default::default() })
                .collect();
        }
        if shows(r.links_visibility) {
            extras.links = self
                .links
                .iter()
                .map(|l| ProfileLink {
                    id: l.id.to_string(),
                    title: l.title.clone(),
                    url: l.url.clone(),
                    verified: l.verified_at.is_some(),
                    verified_at: opt_ts(l.verified_at),
                    ..Default::default()
                })
                .collect();
        }
        if shows(r.long_bio_visibility) {
            extras.long_bio = r.long_bio.clone();
        }
        if shows(r.pinned_song_visibility) {
            extras.pinned_song = r.pinned_song.as_ref().and_then(|v| serde_json::from_value(v.clone()).ok()).into();
        }
        extras.qa_cards = self
            .cards
            .iter()
            .filter(|card| *viewer == Viewer::Owner || (!card.hidden && reaches(card.visibility, card.circle_id, viewer)))
            .map(|card| ProfileQaCard {
                id: card.id.to_string(),
                prompt_id: card.prompt_id.clone(),
                question: card_question(&card.prompt_id, &card.question).into(),
                answer: card.answer.clone(),
                audience: full
                    .then(|| AudienceRef {
                        visibility: card.visibility.into(),
                        circle_id: card.circle_id.map(|c| c.to_string()).unwrap_or_default(),
                        ..Default::default()
                    })
                    .into(),
                hidden: *viewer == Viewer::Owner && card.hidden,
                ..Default::default()
            })
            .collect();
        if full {
            extras.visibility = ProfileFieldVisibility {
                mbti: r.mbti_visibility.into(),
                blood_type: r.blood_type_visibility.into(),
                pronouns: r.pronouns_visibility.into(),
                zodiac: r.zodiac_visibility.into(),
                oshi: r.oshi_visibility.into(),
                tags: r.tags_visibility.into(),
                links: r.links_visibility.into(),
                long_bio: r.long_bio_visibility.into(),
                pinned_song: r.pinned_song_visibility.into(),
                ..Default::default()
            }
            .into();
        }
        extras
    }
}

fn card_question(prompt_id: &str, question: &str) -> LocalizedText {
    match prompt_by_id(prompt_id) {
        Some(p) => LocalizedText { ja: p.ja.to_owned(), en: p.en.to_owned(), ..Default::default() },
        None => LocalizedText { ja: question.to_owned(), en: question.to_owned(), ..Default::default() },
    }
}

// ───────────────────────────── updates ─────────────────────────────

/// An oshi as it will be stored, `image` resolved before the transaction.
struct OshiPlan {
    id: Option<Uuid>,
    name: String,
    /// `None` keeps the image; `Some(None)` clears it; `Some(Some(..))` replaces it.
    image: Option<Option<(Uuid, serde_json::Value)>>,
    url: String,
    emoji: String,
}

struct CardPlan {
    id: Option<Uuid>,
    prompt_id: String,
    question: String,
    answer: String,
    visibility: i32,
    circle_id: Option<Uuid>,
    hidden: bool,
}

fn plan_card(input: &ProfileQaCardInput) -> AppResult<CardPlan> {
    let id = if input.id.is_empty() {
        None
    } else {
        Some(parse_id(&input.id).map_err(|_| AppError::not_found("CARD_NOT_FOUND", "card not found"))?)
    };
    let prompt_id = input.prompt_id.trim().to_owned();
    let question = if prompt_id.is_empty() {
        let question = single_line(&input.question, 60, "CARD_INVALID", "a question")?;
        ensure!(!question.is_empty(), invalid, "CARD_INVALID", "a free card needs a question");
        question
    } else {
        ensure!(prompt_by_id(&prompt_id).is_some(), not_found, "PROMPT_NOT_FOUND", "unknown prompt");
        String::new()
    };
    let answer = input.answer.trim().replace("\r\n", "\n");
    ensure!((1..=200).contains(&graphemes(&answer)), invalid, "CARD_INVALID", "an answer is 1-200 characters");
    ensure!(
        !answer.chars().any(|c| c.is_control() && c != '\n'),
        invalid,
        "CARD_INVALID",
        "the answer contains control characters"
    );
    let audience = input.audience.as_option().cloned().unwrap_or_default();
    let circle_id = if audience.visibility.as_known() == Some(Visibility::VISIBILITY_CIRCLE) {
        // A circle that is not the owner's reaches nobody (the social graph
        // checks owner and member together), so only the form is checked.
        Some(
            parse_id(&audience.circle_id)
                .map_err(|_| AppError::invalid("CARD_INVALID", "a circle card needs one of your circles"))?,
        )
    } else {
        None
    };
    Ok(CardPlan {
        id,
        prompt_id,
        question,
        answer,
        visibility: check_card_audience(&audience.visibility)?,
        circle_id,
        hidden: input.hidden,
    })
}

impl Identity {
    /// The プロフ帳 of `user_id` for its owner.
    pub async fn own_extras(&self, user_id: Uuid, birthdate: Option<NaiveDate>) -> AppResult<ProfileExtras> {
        Ok(Extras::load(&self.core, user_id, birthdate).await?.view(&Viewer::Owner))
    }

    /// `UpdateProfileExtras`: validates everything first (and resolves the
    /// uploads, on the pool), then replaces the groups present on one
    /// transaction that holds the account's row.
    pub(crate) async fn update_profile_extras(
        &self,
        user_id: Uuid,
        req: &UpdateProfileExtrasRequest,
    ) -> AppResult<ProfileExtras> {
        let core = &self.core;
        core.cache.rate_limit(&format!("profile-extras:{user_id}"), 60, Duration::from_secs(3600)).await?;
        let mbti = req.mbti.as_deref().map(validate_mbti).transpose()?;
        let blood_type = match req.blood_type {
            Some(value) => Some(
                value
                    .as_known()
                    .map(|b| b as i32)
                    .ok_or_else(|| AppError::invalid("BLOOD_TYPE_INVALID", "unknown blood type"))?,
            ),
            None => None,
        };
        let pronouns = req.pronouns.as_deref().map(validate_pronouns).transpose()?;
        let long_bio = req.long_bio.as_deref().map(validate_long_bio).transpose()?;
        let theme = req.theme.as_option().map(validate_theme).transpose()?;
        let visibility = match req.visibility.as_option() {
            Some(v) => Some([
                check_visibility(&v.mbti, "mbti")?,
                check_visibility(&v.blood_type, "blood_type")?,
                check_visibility(&v.pronouns, "pronouns")?,
                check_visibility(&v.zodiac, "zodiac")?,
                check_visibility(&v.oshi, "oshi")?,
                check_visibility(&v.tags, "tags")?,
                check_visibility(&v.links, "links")?,
                check_visibility(&v.long_bio, "long_bio")?,
                check_visibility(&v.pinned_song, "pinned_song")?,
            ]),
            None => None,
        };
        let oshi = match req.oshi.as_option() {
            Some(list) => {
                ensure!(list.items.len() <= MAX_OSHI, invalid, "TOO_MANY_OSHI", "at most {MAX_OSHI} 推し");
                let mut plans = Vec::with_capacity(list.items.len());
                for item in &list.items {
                    let id = if item.id.is_empty() {
                        None
                    } else {
                        Some(parse_id(&item.id).map_err(|_| AppError::invalid("OSHI_INVALID", "unknown 推し"))?)
                    };
                    let name = single_line(&item.name, 40, "OSHI_INVALID", "a name")?;
                    ensure!(!name.is_empty(), invalid, "OSHI_INVALID", "every 推し needs a name");
                    let url = if item.url.trim().is_empty() {
                        String::new()
                    } else {
                        validate_link_url(&item.url, "OSHI_INVALID", true)?
                    };
                    let emoji = item.emoji.trim().to_owned();
                    ensure!(
                        graphemes(&emoji) <= 1 && !emoji.chars().any(|c| c.is_ascii_alphanumeric()),
                        invalid,
                        "OSHI_INVALID",
                        "one emoji at most"
                    );
                    let image = match item.image_asset_id.as_deref() {
                        None => None,
                        Some("") => Some(None),
                        Some(asset) => {
                            let asset_id = parse_id(asset)?;
                            let json = assets::require_image_json(
                                core,
                                asset_id,
                                user_id,
                                &[AssetPurpose::ASSET_PURPOSE_AVATAR],
                            )
                            .await?;
                            Some(Some((asset_id, json)))
                        }
                    };
                    plans.push(OshiPlan {
                        id,
                        name,
                        image: if id.is_none() { Some(image.flatten()) } else { image },
                        url,
                        emoji,
                    });
                }
                Some(plans)
            }
            None => None,
        };
        let tags = match req.tags.as_option() {
            Some(list) => {
                ensure!(list.items.len() <= MAX_TAGS, invalid, "TOO_MANY_TAGS", "at most {MAX_TAGS} favourite things");
                let mut seen = HashSet::new();
                let mut tags = Vec::new();
                for tag in &list.items {
                    let (category, name, key) = validate_tag(tag)?;
                    if seen.insert((category, key.clone())) {
                        tags.push((category, name, key));
                    }
                }
                Some(tags)
            }
            None => None,
        };
        let links = match req.links.as_option() {
            Some(list) => {
                ensure!(list.items.len() <= MAX_LINKS, invalid, "TOO_MANY_LINKS", "at most {MAX_LINKS} links");
                let mut seen = HashSet::new();
                let mut links = Vec::new();
                for link in &list.items {
                    let url = validate_link_url(&link.url, "LINK_INVALID", false)?;
                    let title = single_line(&link.title, 40, "LINK_INVALID", "a link title")?;
                    let title = if title.is_empty() {
                        url::Url::parse(&url)
                            .ok()
                            .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_owned()))
                            .unwrap_or_default()
                    } else {
                        title
                    };
                    ensure!(seen.insert(url.clone()), invalid, "LINK_INVALID", "the same link twice");
                    links.push((title, url));
                }
                Some(links)
            }
            None => None,
        };
        let cards = match req.qa_cards.as_option() {
            Some(list) => {
                ensure!(list.items.len() <= MAX_CARDS, invalid, "TOO_MANY_CARDS", "at most {MAX_CARDS} cards");
                let cards = list.items.iter().map(plan_card).collect::<AppResult<Vec<_>>>()?;
                let mut prompts = HashSet::new();
                for card in &cards {
                    ensure!(
                        card.prompt_id.is_empty() || prompts.insert(card.prompt_id.clone()),
                        invalid,
                        "CARD_INVALID",
                        "each prompt is answered once"
                    );
                }
                Some(cards)
            }
            None => None,
        };

        let mut changed: Vec<&str> = Vec::new();
        for (name, present) in [
            ("mbti", mbti.is_some()),
            ("blood_type", blood_type.is_some()),
            ("pronouns", pronouns.is_some()),
            ("long_bio", long_bio.is_some()),
            ("theme", theme.is_some()),
            ("visibility", visibility.is_some()),
            ("oshi", oshi.is_some()),
            ("tags", tags.is_some()),
            ("links", links.is_some()),
            ("qa_cards", cards.is_some()),
        ] {
            if present {
                changed.push(name);
            }
        }

        let mut tx = core.db.begin().await?;
        lock_extras(&mut tx, user_id).await?;
        let theme_json = theme.map(|t| serde_json::to_value(&t).unwrap_or_else(|_| serde_json::json!({})));
        let vis = visibility.map(|v| v.to_vec());
        sqlx::query(
            "UPDATE profile_extras SET
                mbti = COALESCE($2, mbti), blood_type = COALESCE($3, blood_type), pronouns = COALESCE($4, pronouns),
                long_bio = COALESCE($5, long_bio), theme = COALESCE($6, theme),
                mbti_visibility = COALESCE($7[1], mbti_visibility), blood_type_visibility = COALESCE($7[2], blood_type_visibility),
                pronouns_visibility = COALESCE($7[3], pronouns_visibility), zodiac_visibility = COALESCE($7[4], zodiac_visibility),
                oshi_visibility = COALESCE($7[5], oshi_visibility), tags_visibility = COALESCE($7[6], tags_visibility),
                links_visibility = COALESCE($7[7], links_visibility), long_bio_visibility = COALESCE($7[8], long_bio_visibility),
                pinned_song_visibility = COALESCE($7[9], pinned_song_visibility)
             WHERE user_id = $1",
        )
        .bind(user_id)
        .bind(&mbti)
        .bind(blood_type)
        .bind(&pronouns)
        .bind(&long_bio)
        .bind(theme_json)
        .bind(vis)
        .execute(&mut *tx)
        .await?;
        if let Some(oshi) = &oshi {
            replace_oshi(&mut tx, user_id, oshi).await?;
        }
        if let Some(tags) = &tags {
            sqlx::query("DELETE FROM profile_tags WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
            for (position, (category, name, key)) in tags.iter().enumerate() {
                sqlx::query("INSERT INTO profile_tags (user_id, category, name, name_key, position) VALUES ($1, $2, $3, $4, $5)")
                    .bind(user_id)
                    .bind(category)
                    .bind(name)
                    .bind(key)
                    .bind(position as i32)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        let mut links_to_check = false;
        if let Some(links) = &links {
            links_to_check = replace_links(&mut tx, user_id, links).await?;
        }
        if let Some(cards) = &cards {
            replace_cards(&mut tx, user_id, cards).await?;
        }
        if !changed.is_empty() {
            emit_profile_updated(&mut tx, user_id, &changed).await?;
        }
        tx.commit().await?;
        if links_to_check {
            self.enqueue_link_check(user_id).await;
        }
        let birthdate = crate::users::get(core, user_id).await?.birthdate;
        self.own_extras(user_id, birthdate).await
    }

    /// Queues the background check of `user_id`'s links. A failure to queue
    /// is logged: the periodic sweep picks unchecked links up later.
    pub(crate) async fn enqueue_link_check(&self, user_id: Uuid) {
        let job = crate::link_check::LinkCheckJob { user_id };
        if let Err(e) = self.core.jobs.enqueue_unique(LINK_CHECK_JOB, &user_id.to_string(), &job).await {
            tracing::warn!(%user_id, error = %e, "profile link check not queued");
        }
    }

    /// Stores (or clears) the pinned song and announces it.
    pub(crate) async fn store_pinned_song(&self, user_id: Uuid, song: Option<&PinnedSong>) -> AppResult<()> {
        let json = match song {
            Some(song) => {
                Some(serde_json::to_value(song).map_err(|e| AppError::internal(anyhow::anyhow!("song encode: {e}")))?)
            }
            None => None,
        };
        let mut tx = self.core.db.begin().await?;
        lock_extras(&mut tx, user_id).await?;
        sqlx::query("UPDATE profile_extras SET pinned_song = $2 WHERE user_id = $1")
            .bind(user_id)
            .bind(json)
            .execute(&mut *tx)
            .await?;
        emit_profile_updated(&mut tx, user_id, &["pinned_song"]).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Makes sure the account has its `profile_extras` row and holds it for the
/// rest of the transaction: saves of one account run one after the other.
async fn lock_extras(tx: &mut PgConnection, user_id: Uuid) -> AppResult<()> {
    sqlx::query("INSERT INTO profile_extras (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT 1 FROM profile_extras WHERE user_id = $1 FOR UPDATE").bind(user_id).execute(&mut *tx).await?;
    Ok(())
}

async fn emit_profile_updated(tx: &mut PgConnection, user_id: Uuid, changed: &[&str]) -> AppResult<()> {
    let payload = ProfileUpdated { user_id, changed: changed.iter().map(|c| (*c).to_owned()).collect() };
    events::emit(&mut *tx, "identity.profile.updated", Some(user_id), &payload).await?;
    // The account's public face changed: search rebuilds its document on this.
    events::emit(&mut *tx, "identity.user.updated", Some(user_id), &UserRef { user_id }).await?;
    Ok(())
}

async fn replace_oshi(tx: &mut PgConnection, user_id: Uuid, plans: &[OshiPlan]) -> AppResult<()> {
    let existing: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM profile_oshi WHERE user_id = $1").bind(user_id).fetch_all(&mut *tx).await?;
    let kept: HashSet<Uuid> = plans.iter().filter_map(|p| p.id).collect();
    for id in &kept {
        ensure!(existing.contains(id), invalid, "OSHI_INVALID", "unknown 推し");
    }
    for id in existing.iter().filter(|id| !kept.contains(id)) {
        assets::detach_holder(&mut *tx, &oshi_asset_holder(*id)).await?;
        sqlx::query("DELETE FROM profile_oshi WHERE id = $1").bind(id).execute(&mut *tx).await?;
    }
    for (position, plan) in plans.iter().enumerate() {
        let id = match plan.id {
            Some(id) => {
                sqlx::query("UPDATE profile_oshi SET position = $2, name = $3, url = $4, emoji = $5 WHERE id = $1")
                    .bind(id)
                    .bind(position as i32)
                    .bind(&plan.name)
                    .bind(&plan.url)
                    .bind(&plan.emoji)
                    .execute(&mut *tx)
                    .await?;
                id
            }
            None => {
                sqlx::query_scalar("INSERT INTO profile_oshi (user_id, position, name, url, emoji) VALUES ($1, $2, $3, $4, $5) RETURNING id")
                    .bind(user_id)
                    .bind(position as i32)
                    .bind(&plan.name)
                    .bind(&plan.url)
                    .bind(&plan.emoji)
                    .fetch_one(&mut *tx)
                    .await?
            }
        };
        if let Some(image) = &plan.image {
            let holder = oshi_asset_holder(id);
            assets::detach_holder(&mut *tx, &holder).await?;
            if let Some((asset_id, _)) = image {
                assets::attach(&mut *tx, *asset_id, &holder).await?;
            }
            sqlx::query("UPDATE profile_oshi SET image = $2 WHERE id = $1")
                .bind(id)
                .bind(image.as_ref().map(|(_, json)| json.clone()))
                .execute(&mut *tx)
                .await?;
        }
    }
    Ok(())
}

/// Replaces the links; a link whose URL stays keeps its verification.
/// Returns whether some link has not been checked yet.
async fn replace_links(tx: &mut PgConnection, user_id: Uuid, links: &[(String, String)]) -> AppResult<bool> {
    let urls: Vec<String> = links.iter().map(|(_, url)| url.clone()).collect();
    sqlx::query("DELETE FROM profile_links WHERE user_id = $1 AND NOT (url = ANY($2))")
        .bind(user_id)
        .bind(&urls)
        .execute(&mut *tx)
        .await?;
    for (position, (title, url)) in links.iter().enumerate() {
        let updated = sqlx::query("UPDATE profile_links SET position = $3, title = $4 WHERE user_id = $1 AND url = $2")
            .bind(user_id)
            .bind(url)
            .bind(position as i32)
            .bind(title)
            .execute(&mut *tx)
            .await?;
        if updated.rows_affected() == 0 {
            sqlx::query("INSERT INTO profile_links (user_id, position, title, url) VALUES ($1, $2, $3, $4)")
                .bind(user_id)
                .bind(position as i32)
                .bind(title)
                .bind(url)
                .execute(&mut *tx)
                .await?;
        }
    }
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM profile_links WHERE user_id = $1 AND checked_at IS NULL)")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?)
}

async fn replace_cards(tx: &mut PgConnection, user_id: Uuid, cards: &[CardPlan]) -> AppResult<()> {
    let existing: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM profile_qa_cards WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
    let kept: HashSet<Uuid> = cards.iter().filter_map(|c| c.id).collect();
    for id in &kept {
        ensure!(existing.contains(id), not_found, "CARD_NOT_FOUND", "card not found");
    }
    sqlx::query("DELETE FROM profile_qa_cards WHERE user_id = $1 AND NOT (id = ANY($2))")
        .bind(user_id)
        .bind(kept.iter().copied().collect::<Vec<_>>())
        .execute(&mut *tx)
        .await?;
    for (position, card) in cards.iter().enumerate() {
        match card.id {
            Some(id) => {
                sqlx::query(
                    "UPDATE profile_qa_cards SET position = $2, prompt_id = $3, question = $4, answer = $5, visibility = $6, hidden = $7, circle_id = $8
                     WHERE id = $1",
                )
                .bind(id)
                .bind(position as i32)
                .bind(&card.prompt_id)
                .bind(&card.question)
                .bind(&card.answer)
                .bind(card.visibility)
                .bind(card.hidden)
                .bind(card.circle_id)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query(
                    "INSERT INTO profile_qa_cards (user_id, position, prompt_id, question, answer, visibility, hidden, circle_id)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                )
                .bind(user_id)
                .bind(position as i32)
                .bind(&card.prompt_id)
                .bind(&card.question)
                .bind(&card.answer)
                .bind(card.visibility)
                .bind(card.hidden)
                .bind(card.circle_id)
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    Ok(())
}

/// Removes the whole プロフ帳 of an account being erased (and lets go of
/// the 推し images), on the purge's transaction.
pub(crate) async fn erase(tx: &mut PgConnection, user_id: Uuid) -> AppResult<()> {
    let oshi: Vec<Uuid> = sqlx::query_scalar("DELETE FROM profile_oshi WHERE user_id = $1 RETURNING id")
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
    for id in oshi {
        assets::detach_holder(&mut *tx, &oshi_asset_holder(id)).await?;
    }
    for table in ["profile_tags", "profile_links", "profile_qa_cards", "profile_extras"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = $1")))
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

// ───────────────────────────── search & suggestions ─────────────────────────────

/// What of an account's プロフ帳 its search document carries: only public
/// fields, for search is public.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileSearchTerms {
    /// The public long bio.
    pub text: String,
    /// Public favourite things and 推し.
    pub tags: Vec<String>,
}

/// [`ProfileSearchTerms`] for many accounts at once (for the search
/// domain's user documents). Accounts with nothing public are absent, and so
/// are the `locked` ones (the caller asks the social graph which those are).
pub async fn search_terms(
    core: &Core,
    user_ids: &[Uuid],
    locked: &HashSet<Uuid>,
) -> AppResult<HashMap<Uuid, ProfileSearchTerms>> {
    // A locked account's public fields reach only its followers and friends.
    let user_ids: Vec<Uuid> = user_ids.iter().copied().filter(|id| !locked.contains(id)).collect();
    let user_ids = user_ids.as_slice();
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let public = Visibility::VISIBILITY_PUBLIC as i32;
    let bios: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT user_id, long_bio FROM profile_extras WHERE user_id = ANY($1) AND long_bio <> '' AND long_bio_visibility = $2",
    )
    .bind(user_ids)
    .bind(public)
    .fetch_all(&core.db)
    .await?;
    let names: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT t.user_id, t.name FROM profile_tags t JOIN profile_extras e ON e.user_id = t.user_id AND e.tags_visibility = $2
         WHERE t.user_id = ANY($1)
         UNION ALL
         SELECT o.user_id, o.name FROM profile_oshi o JOIN profile_extras e ON e.user_id = o.user_id AND e.oshi_visibility = $2
         WHERE o.user_id = ANY($1)",
    )
    .bind(user_ids)
    .bind(public)
    .fetch_all(&core.db)
    .await?;
    let mut terms: HashMap<Uuid, ProfileSearchTerms> = HashMap::new();
    for (id, bio) in bios {
        terms.entry(id).or_default().text = bio.chars().take(1000).collect();
    }
    for (id, name) in names {
        terms.entry(id).or_default().tags.push(name);
    }
    Ok(terms)
}

/// How many people must show a tag publicly before it is suggested: a tag
/// only one person uses would point at them.
pub const TAG_SUGGESTION_MIN_PEOPLE: usize = 5;

impl Identity {
    /// Favourite things people show publicly in `category`, by how many show
    /// them, beginning with `query` (folded like [`tag_key`]). Only tags at
    /// least [`TAG_SUGGESTION_MIN_PEOPLE`] people show; suspended, deleted,
    /// bot and locked accounts do not count.
    pub(crate) async fn suggest_tags(&self, category: i32, query: &str) -> AppResult<Vec<TagSuggestion>> {
        let key = tag_key(query);
        let pattern = format!("{}%", key.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        // Counted in the database first: the tags enough people show, most
        // shown first (a bound on rows read would cut off whatever sorts
        // after a popular beginning). Locked accounts only lower a count, so
        // a tag below the floor here stays below it.
        const CANDIDATE_TAGS: i64 = 200;
        let keys: Vec<String> = sqlx::query_scalar(
            "SELECT t.name_key FROM profile_tags t
             JOIN profile_extras e ON e.user_id = t.user_id AND e.tags_visibility = $3
             JOIN users u ON u.id = t.user_id AND u.deleted_at IS NULL AND u.banned_at IS NULL AND NOT u.bot
             WHERE t.category = $1 AND t.name_key LIKE $2
             GROUP BY t.name_key HAVING count(DISTINCT t.user_id) >= $4
             ORDER BY count(DISTINCT t.user_id) DESC, t.name_key LIMIT $5",
        )
        .bind(category)
        .bind(&pattern)
        .bind(Visibility::VISIBILITY_PUBLIC as i32)
        .bind(TAG_SUGGESTION_MIN_PEOPLE as i64)
        .bind(CANDIDATE_TAGS)
        .fetch_all(&self.core.db)
        .await?;
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        // Who shows them, to take the locked accounts out.
        let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
            "SELECT t.user_id, t.name_key, t.name FROM profile_tags t
             JOIN profile_extras e ON e.user_id = t.user_id AND e.tags_visibility = $3
             JOIN users u ON u.id = t.user_id AND u.deleted_at IS NULL AND u.banned_at IS NULL AND NOT u.bot
             WHERE t.category = $1 AND t.name_key = ANY($2)",
        )
        .bind(category)
        .bind(&keys)
        .bind(Visibility::VISIBILITY_PUBLIC as i32)
        .fetch_all(&self.core.db)
        .await?;
        let mut owners: Vec<Uuid> = rows.iter().map(|(user, _, _)| *user).collect();
        owners.sort_unstable();
        owners.dedup();
        let locked = self.locked_among(&owners).await?;
        let mut by_key: HashMap<String, (HashSet<Uuid>, HashMap<String, usize>)> = HashMap::new();
        for (user, key, name) in rows {
            if locked.contains(&user) {
                continue;
            }
            let entry = by_key.entry(key).or_default();
            entry.0.insert(user);
            *entry.1.entry(name).or_default() += 1;
        }
        let mut suggestions: Vec<(usize, String, String)> = by_key
            .into_iter()
            .filter(|(_, (people, _))| people.len() >= TAG_SUGGESTION_MIN_PEOPLE)
            .map(|(key, (people, spellings))| {
                // The spelling most of them use.
                let name = spellings
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                    .map(|(name, _)| name)
                    .unwrap_or_default();
                (people.len(), key, name)
            })
            .collect();
        suggestions.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        Ok(suggestions
            .into_iter()
            .take(20)
            .map(|(count, _, name)| TagSuggestion { name, count: count as i32, ..Default::default() })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mbti_is_four_letters_with_an_optional_variant() {
        assert_eq!(validate_mbti(" enfp ").unwrap(), "ENFP");
        assert_eq!(validate_mbti("INFJ-t").unwrap(), "INFJ-T");
        assert_eq!(validate_mbti("").unwrap(), "");
        for bad in ["ENF", "XNFP", "ENFP-X", "ENFPA", "あいうえ"] {
            assert_eq!(validate_mbti(bad).unwrap_err().reason, "MBTI_INVALID", "{bad}");
        }
    }

    #[test]
    fn zodiac_follows_the_cusps() {
        let d = |m, d| NaiveDate::from_ymd_opt(2008, m, d).unwrap();
        assert_eq!(zodiac_of(d(1, 19)), Zodiac::ZODIAC_CAPRICORN);
        assert_eq!(zodiac_of(d(1, 20)), Zodiac::ZODIAC_AQUARIUS);
        assert_eq!(zodiac_of(d(3, 21)), Zodiac::ZODIAC_ARIES);
        assert_eq!(zodiac_of(d(7, 7)), Zodiac::ZODIAC_CANCER);
        assert_eq!(zodiac_of(d(12, 22)), Zodiac::ZODIAC_CAPRICORN);
        assert_eq!(zodiac_of(d(12, 21)), Zodiac::ZODIAC_SAGITTARIUS);
    }

    #[test]
    fn tags_fold_width_and_case() {
        assert_eq!(tag_key("ﾖﾙｼｶ"), tag_key("ヨルシカ"));
        assert_eq!(tag_key("ＡＰＥＸ  Legends"), "apex legends");
    }

    #[test]
    fn long_bio_keeps_lines_and_folds_blank_runs() {
        assert_eq!(validate_long_bio("a\r\nb\n\n\n\n\nc  ").unwrap(), "a\nb\n\n\nc");
        assert_eq!(validate_long_bio(&"あ".repeat(2001)).unwrap_err().reason, "LONG_BIO_TOO_LONG");
        assert!(validate_long_bio("bell\u{7}").is_err());
    }

    #[test]
    fn audiences_reach_the_right_people() {
        use Visibility::*;
        let v = |x: Visibility| x as i32;
        let tie = |tier: Tier, locked: bool| {
            Viewer::Audience(AudienceTie { tier, owner_locked: locked, ..Default::default() })
        };
        let (stranger, follower, friend) =
            (tie(Tier::Other, false), tie(Tier::Follower, false), tie(Tier::Friend, false));
        assert!(reaches(v(VISIBILITY_PUBLIC), None, &stranger));
        assert!(
            !reaches(v(VISIBILITY_FOLLOWERS), None, &stranger) && reaches(v(VISIBILITY_FOLLOWERS), None, &follower)
        );
        assert!(reaches(v(VISIBILITY_FOLLOWERS), None, &friend), "friends are inside followers");
        assert!(!reaches(v(VISIBILITY_FRIENDS), None, &follower) && reaches(v(VISIBILITY_FRIENDS), None, &friend));
        assert!(!reaches(v(VISIBILITY_PRIVATE), None, &friend));
        assert!(reaches(v(VISIBILITY_PRIVATE), None, &Viewer::Owner));
        // A moderator sees what reached anybody, not what the owner keeps to themself.
        assert!(!reaches(v(VISIBILITY_PRIVATE), None, &Viewer::Moderator));
        assert!(reaches(v(VISIBILITY_CLOSE_FRIENDS), None, &Viewer::Moderator));
        assert!(!reaches(0, None, &Viewer::Moderator));
        // A locked account's public parts reach followers and friends only.
        assert!(!reaches(v(VISIBILITY_PUBLIC), None, &tie(Tier::Other, true)));
        assert!(reaches(v(VISIBILITY_PUBLIC), None, &tie(Tier::Follower, true)));
        // Close friends and circles: tied, and in them.
        let circle = Uuid::now_v7();
        let close = Viewer::Audience(AudienceTie {
            tier: Tier::Follower,
            close_friend: true,
            circles: vec![circle],
            ..Default::default()
        });
        assert!(
            reaches(v(VISIBILITY_CLOSE_FRIENDS), None, &close) && !reaches(v(VISIBILITY_CLOSE_FRIENDS), None, &friend)
        );
        assert!(reaches(v(VISIBILITY_CIRCLE), Some(circle), &close));
        assert!(!reaches(v(VISIBILITY_CIRCLE), Some(Uuid::now_v7()), &close));
        assert!(!reaches(v(VISIBILITY_CIRCLE), None, &close));
        let untied = Viewer::Audience(AudienceTie { close_friend: true, circles: vec![circle], ..Default::default() });
        assert!(!reaches(v(VISIBILITY_CIRCLE), Some(circle), &untied), "a circle needs a tie that still holds");
        // Anything not on the list reaches nobody but the owner.
        for unlisted in [0, v(VISIBILITY_NO_FEED), v(VISIBILITY_UNLISTED), 99] {
            assert!(!reaches(unlisted, None, &friend), "{unlisted}");
        }
    }

    #[test]
    fn links_must_be_web_addresses() {
        assert_eq!(
            validate_link_url("https://example.com/me", "LINK_INVALID", false).unwrap(),
            "https://example.com/me"
        );
        for bad in [
            "javascript:alert(1)",
            "example.com",
            "https://user:pw@example.com",
            "ftp://example.com",
            "https://localhost",
        ] {
            assert!(validate_link_url(bad, "LINK_INVALID", false).is_err(), "{bad}");
        }
        assert!(validate_link_url("http://example.com", "OSHI_INVALID", true).is_err());
    }

    #[test]
    fn hidden_and_private_cards_stay_with_the_owner() {
        let card = |visibility: Visibility, hidden: bool| CardRow {
            id: Uuid::now_v7(),
            prompt_id: "fav_food".into(),
            question: String::new(),
            answer: "ラーメン".into(),
            visibility: visibility as i32,
            circle_id: None,
            hidden,
        };
        let extras = Extras {
            cards: vec![
                card(Visibility::VISIBILITY_PUBLIC, false),
                card(Visibility::VISIBILITY_FRIENDS, false),
                card(Visibility::VISIBILITY_PRIVATE, false),
                card(Visibility::VISIBILITY_PUBLIC, true),
            ],
            row: ExtrasRow {
                mbti: "ENFP".into(),
                mbti_visibility: Visibility::VISIBILITY_FRIENDS as i32,
                ..Default::default()
            },
            birthdate: NaiveDate::from_ymd_opt(2008, 7, 7),
            ..Default::default()
        };
        let other = extras.view(&Viewer::stranger());
        assert_eq!(other.qa_cards.len(), 1);
        assert_eq!(other.mbti, "");
        assert_eq!(other.zodiac, Zodiac::ZODIAC_UNSPECIFIED, "the zodiac starts private");
        assert!(other.visibility.as_option().is_none());
        let friend = extras.view(&Viewer::Audience(AudienceTie { tier: Tier::Friend, ..Default::default() }));
        assert_eq!(friend.qa_cards.len(), 2);
        assert_eq!(friend.mbti, "ENFP");
        let owner = extras.view(&Viewer::Owner);
        assert_eq!(owner.qa_cards.len(), 4);
        assert_eq!(owner.zodiac, Zodiac::ZODIAC_CANCER);
        assert!(owner.qa_cards[3].hidden);
        assert_eq!(owner.qa_cards[0].question.as_option().unwrap().ja, "好きな食べ物");
    }

    #[test]
    fn prompt_ids_are_unique() {
        let ids: HashSet<&str> = PROMPTS.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), PROMPTS.len());
    }
}
