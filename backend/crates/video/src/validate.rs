//! Validation of everything users type: names, titles, tags, chapters,
//! links, language tags. Pure functions; lengths are counted in grapheme
//! clusters, the way people count characters.
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::Visibility,
    video::{ChannelLink, Chapter, VideoCategory},
};
use unicode_segmentation::UnicodeSegmentation as _;

pub const MAX_CHANNEL_NAME: usize = 50;
pub const MAX_CHANNEL_DESCRIPTION: usize = 1000;
pub const MAX_CHANNEL_LINKS: usize = 5;
pub const MAX_LINK_TITLE: usize = 30;
pub const MAX_LINK_URL: usize = 500;
pub const MAX_TITLE: usize = 100;
pub const MAX_DESCRIPTION: usize = 5000;
pub const MAX_TAGS: usize = 20;
pub const MAX_TAG: usize = 30;
pub const MAX_GAME: usize = 100;
pub const MAX_CHAPTERS: usize = 100;
pub const MAX_CHAPTER_TITLE: usize = 100;
/// Shortest chapter, in seconds.
pub const MIN_CHAPTER_SECONDS: f64 = 5.0;
pub const MAX_PLAYLIST_TITLE: usize = 100;
pub const MAX_PLAYLIST_DESCRIPTION: usize = 1000;
pub const MAX_SUBTITLE_LABEL: usize = 40;
pub const MAX_GUEST_NAME: usize = 24;
pub const MAX_PARTY_MESSAGE: usize = 500;

fn length(text: &str) -> usize {
    text.graphemes(true).count()
}

fn has_control(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// Like [`has_control`], but line breaks and tabs are fine (multi-line text).
fn has_control_in_text(text: &str) -> bool {
    text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

/// A channel handle: the same rules (and reserved words) as user handles.
pub fn handle(input: &str) -> AppResult<String> {
    tokuly_identity::users::validate_handle(input)
}

pub fn channel_name(input: &str) -> AppResult<String> {
    let name = input.trim();
    ensure!(
        (1..=MAX_CHANNEL_NAME).contains(&length(name)) && !has_control(name),
        invalid,
        "CHANNEL_NAME_INVALID",
        "channel names are 1-{MAX_CHANNEL_NAME} characters"
    );
    Ok(name.to_owned())
}

pub fn channel_description(input: &str) -> AppResult<String> {
    let description = input.trim();
    ensure!(
        length(description) <= MAX_CHANNEL_DESCRIPTION && !has_control_in_text(description),
        invalid,
        "CHANNEL_DESCRIPTION_TOO_LONG",
        "channel descriptions are at most {MAX_CHANNEL_DESCRIPTION} characters"
    );
    Ok(description.to_owned())
}

/// Whether `input` is an absolute http(s) URL of a sane length.
fn is_web_url(input: &str) -> bool {
    input.len() <= MAX_LINK_URL
        && url::Url::parse(input).is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

pub fn channel_links(links: &[ChannelLink]) -> AppResult<Vec<ChannelLink>> {
    ensure!(links.len() <= MAX_CHANNEL_LINKS, invalid, "CHANNEL_LINKS_INVALID", "at most {MAX_CHANNEL_LINKS} links");
    links
        .iter()
        .map(|link| {
            let title = link.title.trim();
            let url = link.url.trim();
            ensure!(
                (1..=MAX_LINK_TITLE).contains(&length(title)) && !has_control(title) && is_web_url(url),
                invalid,
                "CHANNEL_LINKS_INVALID",
                "a link needs a title of 1-{MAX_LINK_TITLE} characters and an http(s) URL"
            );
            Ok(ChannelLink { title: title.to_owned(), url: url.to_owned(), ..Default::default() })
        })
        .collect()
}

pub fn title(input: &str) -> AppResult<String> {
    let title = input.trim();
    ensure!(
        (1..=MAX_TITLE).contains(&length(title)) && !has_control(title),
        invalid,
        "TITLE_INVALID",
        "titles are 1-{MAX_TITLE} characters"
    );
    Ok(title.to_owned())
}

/// A title for a video created without one: the upload's file name without
/// its extension, cut to fit.
pub fn title_from_file_name(file_name: &str) -> String {
    let stem = file_name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(file_name);
    let cleaned: String = stem.chars().filter(|c| !c.is_control()).collect();
    let cut: String = cleaned.trim().graphemes(true).take(MAX_TITLE).collect();
    if cut.is_empty() { "無題の動画".to_owned() } else { cut }
}

pub fn description(input: &str) -> AppResult<String> {
    let description = input.trim();
    ensure!(
        length(description) <= MAX_DESCRIPTION && !has_control_in_text(description),
        invalid,
        "DESCRIPTION_TOO_LONG",
        "descriptions are at most {MAX_DESCRIPTION} characters"
    );
    Ok(description.to_owned())
}

/// Normalizes tags: `#` stripped, lowercased, duplicates dropped (the first
/// spelling's position is kept).
pub fn tags<S: AsRef<str>>(inputs: &[S]) -> AppResult<Vec<String>> {
    let mut tags: Vec<String> = Vec::new();
    for input in inputs {
        let tag = input.as_ref().trim().trim_start_matches(['#', '＃']).trim().to_lowercase();
        if tag.is_empty() {
            continue;
        }
        ensure!(
            length(&tag) <= MAX_TAG
                && !tag.chars().any(|c| c.is_control() || c.is_whitespace() || c == '#' || c == ','),
            invalid,
            "TAGS_INVALID",
            "tags are at most {MAX_TAG} characters without spaces"
        );
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    ensure!(tags.len() <= MAX_TAGS, invalid, "TAGS_INVALID", "at most {MAX_TAGS} tags");
    Ok(tags)
}

pub fn game(input: &str) -> AppResult<String> {
    let game = input.split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!(
        length(&game) <= MAX_GAME && !has_control(&game),
        invalid,
        "GAME_INVALID",
        "game titles are at most {MAX_GAME} characters"
    );
    Ok(game)
}

/// A category as stored: a known `VideoCategory`, or 0 for none.
pub fn category(value: buffa::EnumValue<VideoCategory>) -> AppResult<i32> {
    ensure!(value.as_known().is_some(), invalid, "CATEGORY_INVALID", "unknown category");
    Ok(value.to_i32())
}

/// A video visibility as stored; `default` when unspecified.
pub fn visibility(value: buffa::EnumValue<Visibility>, default: Visibility) -> AppResult<i32> {
    match value.as_known() {
        Some(Visibility::VISIBILITY_UNSPECIFIED) => Ok(default as i32),
        Some(known) => Ok(known as i32),
        None => Err(AppError::invalid("VISIBILITY_INVALID", "unknown visibility")),
    }
}

/// A playlist visibility as stored: PUBLIC, UNLISTED or PRIVATE (the default).
pub fn playlist_visibility(value: buffa::EnumValue<Visibility>) -> AppResult<i32> {
    match value.as_known() {
        Some(Visibility::VISIBILITY_UNSPECIFIED) => Ok(Visibility::VISIBILITY_PRIVATE as i32),
        Some(
            known @ (Visibility::VISIBILITY_PUBLIC | Visibility::VISIBILITY_UNLISTED | Visibility::VISIBILITY_PRIVATE),
        ) => Ok(known as i32),
        _ => Err(AppError::invalid("VISIBILITY_INVALID", "playlists are public, unlisted or private")),
    }
}

/// Validates chapters and returns them sorted by start. An empty list is
/// fine; otherwise the first chapter starts at 0, chapters are at least
/// [`MIN_CHAPTER_SECONDS`] apart and start before the video ends
/// (`duration` of `None`: not known yet).
pub fn chapters(inputs: &[Chapter], duration: Option<f64>) -> AppResult<Vec<Chapter>> {
    let bad = |message: &str| AppError::invalid("CHAPTERS_INVALID", message.to_owned());
    if inputs.len() > MAX_CHAPTERS {
        return Err(bad("too many chapters"));
    }
    let mut chapters = Vec::with_capacity(inputs.len());
    for input in inputs {
        let title = input.title.trim();
        if !(1..=MAX_CHAPTER_TITLE).contains(&length(title)) || has_control(title) {
            return Err(bad("chapter titles are 1-100 characters"));
        }
        if !input.start_seconds.is_finite() || input.start_seconds < 0.0 {
            return Err(bad("chapter start times must be non-negative"));
        }
        chapters.push(Chapter { start_seconds: input.start_seconds, title: title.to_owned(), ..Default::default() });
    }
    chapters.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    if chapters.first().is_some_and(|first| first.start_seconds != 0.0) {
        return Err(bad("the first chapter must start at 0"));
    }
    for pair in chapters.windows(2) {
        if pair[1].start_seconds - pair[0].start_seconds < MIN_CHAPTER_SECONDS {
            return Err(bad("chapters must be at least 5 seconds apart"));
        }
    }
    if let (Some(last), Some(duration)) = (chapters.last(), duration)
        && chapters.len() > 1
        && last.start_seconds >= duration
    {
        return Err(bad("chapters must start before the video ends"));
    }
    Ok(chapters)
}

pub fn playlist_title(input: &str) -> AppResult<String> {
    let title = input.trim();
    ensure!(
        (1..=MAX_PLAYLIST_TITLE).contains(&length(title)) && !has_control(title),
        invalid,
        "PLAYLIST_TITLE_INVALID",
        "playlist titles are 1-{MAX_PLAYLIST_TITLE} characters"
    );
    Ok(title.to_owned())
}

pub fn playlist_description(input: &str) -> AppResult<String> {
    let description = input.trim();
    ensure!(
        length(description) <= MAX_PLAYLIST_DESCRIPTION && !has_control_in_text(description),
        invalid,
        "PLAYLIST_DESCRIPTION_TOO_LONG",
        "playlist descriptions are at most {MAX_PLAYLIST_DESCRIPTION} characters"
    );
    Ok(description.to_owned())
}

/// Normalizes a BCP 47 language tag the way browsers expect it in
/// `<track srclang>`: language lowercase, script title case, region upper
/// case (`ja`, `zh-Hant`, `pt-BR`).
pub fn subtitle_language(input: &str) -> AppResult<String> {
    let bad = || AppError::invalid("SUBTITLE_LANGUAGE_INVALID", "use a language tag such as ja, en or pt-BR");
    let input = input.trim().replace('_', "-");
    let mut parts = input.split('-');
    let language = parts.next().unwrap_or_default();
    if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(bad());
    }
    let mut tag = language.to_ascii_lowercase();
    for part in parts {
        if !(2..=8).contains(&part.len()) || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(bad());
        }
        let alphabetic = part.bytes().all(|b| b.is_ascii_alphabetic());
        let part = match part.len() {
            2 if alphabetic => part.to_ascii_uppercase(),
            4 if alphabetic => {
                let lower = part.to_ascii_lowercase();
                format!("{}{}", lower[..1].to_ascii_uppercase(), &lower[1..])
            }
            _ => part.to_ascii_lowercase(),
        };
        tag.push('-');
        tag.push_str(&part);
    }
    if tag.len() > 35 {
        return Err(bad());
    }
    Ok(tag)
}

/// A track label; empty means "use the language tag".
pub fn subtitle_label(input: &str, language: &str) -> AppResult<String> {
    let label = input.trim();
    if label.is_empty() {
        return Ok(language.to_owned());
    }
    ensure!(
        length(label) <= MAX_SUBTITLE_LABEL && !has_control(label),
        invalid,
        "SUBTITLE_LABEL_INVALID",
        "subtitle labels are 1-{MAX_SUBTITLE_LABEL} characters"
    );
    Ok(label.to_owned())
}

/// The display name a guest chose for a watch party.
pub fn guest_name(input: &str) -> AppResult<String> {
    let name = input.split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!(
        (1..=MAX_GUEST_NAME).contains(&length(&name)) && !has_control(&name),
        invalid,
        "GUEST_NAME_INVALID",
        "guest names are 1-{MAX_GUEST_NAME} characters"
    );
    Ok(name)
}

/// The text of a party chat message (may be empty when a stamp is sent).
pub fn party_text(input: &str) -> AppResult<String> {
    let text = input.trim();
    ensure!(
        length(text) <= MAX_PARTY_MESSAGE && !has_control_in_text(text),
        invalid,
        "MESSAGE_TOO_LONG",
        "messages are at most {MAX_PARTY_MESSAGE} characters"
    );
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(start: f64, title: &str) -> Chapter {
        Chapter { start_seconds: start, title: title.to_owned(), ..Default::default() }
    }

    #[test]
    fn names_count_graphemes_and_reject_control_characters() {
        assert_eq!(channel_name("  とくぞうch 👨‍👩‍👧  ").unwrap(), "とくぞうch 👨‍👩‍👧");
        assert!(channel_name(&"あ".repeat(50)).is_ok());
        assert_eq!(channel_name(&"あ".repeat(51)).unwrap_err().reason, "CHANNEL_NAME_INVALID");
        assert_eq!(channel_name("   ").unwrap_err().reason, "CHANNEL_NAME_INVALID");
        assert_eq!(channel_name("a\u{7}b").unwrap_err().reason, "CHANNEL_NAME_INVALID");
        assert_eq!(title(&"x".repeat(101)).unwrap_err().reason, "TITLE_INVALID");
        assert_eq!(title(" 初投稿! ").unwrap(), "初投稿!");
        assert!(description("line one\nline two").is_ok());
        assert_eq!(description(&"d".repeat(5001)).unwrap_err().reason, "DESCRIPTION_TOO_LONG");
    }

    #[test]
    fn links_need_a_title_and_a_web_url() {
        let link = |title: &str, url: &str| ChannelLink { title: title.into(), url: url.into(), ..Default::default() };
        let ok = channel_links(&[link(" X ", " https://x.com/tokuly ")]).unwrap();
        assert_eq!((ok[0].title.as_str(), ok[0].url.as_str()), ("X", "https://x.com/tokuly"));
        for bad in [
            link("", "https://example.com"),
            link("site", "javascript:alert(1)"),
            link("site", "ftp://example.com"),
            link("site", "not a url"),
        ] {
            assert_eq!(channel_links(&[bad]).unwrap_err().reason, "CHANNEL_LINKS_INVALID");
        }
        let many: Vec<ChannelLink> = (0..6).map(|i| link("a", &format!("https://example.com/{i}"))).collect();
        assert_eq!(channel_links(&many).unwrap_err().reason, "CHANNEL_LINKS_INVALID");
    }

    #[test]
    fn tags_are_normalized_and_deduplicated() {
        assert_eq!(
            tags(&["#Minecraft", "minecraft", " ＃実況 ", "", "ASMR"]).unwrap(),
            vec!["minecraft", "実況", "asmr"]
        );
        assert_eq!(tags(&["two words"]).unwrap_err().reason, "TAGS_INVALID");
        assert_eq!(tags(&["x".repeat(31)]).unwrap_err().reason, "TAGS_INVALID");
        let many: Vec<String> = (0..21).map(|i| format!("tag{i}")).collect();
        assert_eq!(tags(&many).unwrap_err().reason, "TAGS_INVALID");
    }

    #[test]
    fn titles_fall_back_to_the_file_name() {
        assert_eq!(title_from_file_name("夏休み vlog.final.mp4"), "夏休み vlog.final");
        assert_eq!(title_from_file_name(".mp4"), "無題の動画");
        assert_eq!(title_from_file_name(&format!("{}.mov", "長".repeat(150))).graphemes(true).count(), MAX_TITLE);
    }

    #[test]
    fn chapters_must_start_at_zero_be_spaced_and_inside_the_video() {
        let sorted = chapters(&[chapter(60.0, " 本編 "), chapter(0.0, "OP")], Some(120.0)).unwrap();
        assert_eq!(sorted.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), vec!["OP", "本編"]);
        assert!(chapters(&[], Some(10.0)).unwrap().is_empty());
        let reason = |list: &[Chapter], duration| chapters(list, duration).unwrap_err().reason;
        assert_eq!(reason(&[chapter(3.0, "late")], Some(60.0)), "CHAPTERS_INVALID");
        assert_eq!(reason(&[chapter(0.0, "a"), chapter(4.0, "b")], Some(60.0)), "CHAPTERS_INVALID");
        assert_eq!(reason(&[chapter(0.0, "a"), chapter(60.0, "b")], Some(60.0)), "CHAPTERS_INVALID");
        assert_eq!(reason(&[chapter(0.0, "")], None), "CHAPTERS_INVALID");
        assert_eq!(reason(&[chapter(0.0, "a"), chapter(f64::NAN, "b")], None), "CHAPTERS_INVALID");
        // The length is not known while the upload is processing.
        assert!(chapters(&[chapter(0.0, "a"), chapter(600.0, "b")], None).is_ok());
    }

    #[test]
    fn language_tags_are_normalized() {
        assert_eq!(subtitle_language(" JA ").unwrap(), "ja");
        assert_eq!(subtitle_language("pt_br").unwrap(), "pt-BR");
        assert_eq!(subtitle_language("ZH-hant-tw").unwrap(), "zh-Hant-TW");
        for bad in ["", "j", "japanese", "ja-", "ja-!", "日本語"] {
            assert_eq!(subtitle_language(bad).unwrap_err().reason, "SUBTITLE_LANGUAGE_INVALID", "{bad}");
        }
        assert_eq!(subtitle_label("", "ja").unwrap(), "ja");
        assert_eq!(subtitle_label(" 日本語 ", "ja").unwrap(), "日本語");
        assert_eq!(subtitle_label(&"x".repeat(41), "ja").unwrap_err().reason, "SUBTITLE_LABEL_INVALID");
    }

    #[test]
    fn enums_are_checked() {
        use buffa::EnumValue;
        assert_eq!(visibility(Visibility::VISIBILITY_UNSPECIFIED.into(), Visibility::VISIBILITY_PRIVATE).unwrap(), 5);
        assert_eq!(visibility(Visibility::VISIBILITY_FRIENDS.into(), Visibility::VISIBILITY_PRIVATE).unwrap(), 4);
        assert_eq!(
            visibility(EnumValue::from(42), Visibility::VISIBILITY_PRIVATE).unwrap_err().reason,
            "VISIBILITY_INVALID"
        );
        assert_eq!(playlist_visibility(Visibility::VISIBILITY_UNSPECIFIED.into()).unwrap(), 5);
        assert_eq!(
            playlist_visibility(Visibility::VISIBILITY_FRIENDS.into()).unwrap_err().reason,
            "VISIBILITY_INVALID"
        );
        assert_eq!(category(VideoCategory::VIDEO_CATEGORY_GAMING.into()).unwrap(), 1);
        assert_eq!(category(EnumValue::from(99)).unwrap_err().reason, "CATEGORY_INVALID");
    }

    #[test]
    fn guest_names_and_messages_are_bounded() {
        assert_eq!(guest_name("  ゲスト   太郎 ").unwrap(), "ゲスト 太郎");
        assert_eq!(guest_name("").unwrap_err().reason, "GUEST_NAME_INVALID");
        assert_eq!(guest_name(&"g".repeat(25)).unwrap_err().reason, "GUEST_NAME_INVALID");
        assert_eq!(party_text(&"あ".repeat(500)).unwrap().graphemes(true).count(), 500);
        assert_eq!(party_text(&"あ".repeat(501)).unwrap_err().reason, "MESSAGE_TOO_LONG");
        assert_eq!(game("  Apex   Legends ").unwrap(), "Apex Legends");
    }
}
