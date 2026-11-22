//! Validation of what people type and pick: names, descriptions, tags,
//! rules, questions, polls, event times. Each function returns the value as
//! it is stored, or an `invalid` error with the reason the API documents.
use buffa::EnumValue;
use chrono::Duration;
use tokuly_core::prelude::*;
use tokuly_proto::community::{self as pb, CommunityCategory, CommunityVisibility, NotificationLevel, PostingPolicy};

use crate::text;

pub const MAX_NAME_CHARS: usize = 50;
pub const MAX_DESCRIPTION_CHARS: usize = 1000;
pub const MAX_TAGS: usize = 10;
pub const MAX_TAG_CHARS: usize = 30;
pub const MAX_RULES: usize = 20;
pub const MAX_RULE_TITLE_CHARS: usize = 80;
pub const MAX_RULE_BODY_CHARS: usize = 500;
pub const MAX_QUESTIONS: usize = 5;
pub const MAX_QUESTION_CHARS: usize = 200;
pub const MAX_ANSWER_CHARS: usize = 500;
pub const MAX_CHANNEL_NAME_CHARS: usize = 30;
pub const MAX_CHANNEL_DESCRIPTION_CHARS: usize = 200;
pub const MAX_TITLE_CHARS: usize = 100;
pub const MAX_THREAD_BODY_CHARS: usize = 5000;
pub const MAX_REPLY_BODY_CHARS: usize = 2000;
pub const MAX_THREAD_TAGS: usize = 5;
pub const MAX_IMAGES: usize = 4;
pub const MAX_REASON_CHARS: usize = 300;
pub const MAX_POLL_OPTIONS: usize = 6;
pub const MAX_POLL_OPTION_CHARS: usize = 80;
pub const MAX_MUTED_WORDS: usize = 200;
pub const MAX_MUTED_WORD_CHARS: usize = 40;
pub const MAX_QUERY_CHARS: usize = 64;
pub const MAX_INVITE_NOTE_CHARS: usize = 100;
pub const MAX_INVITE_SECONDS: i32 = 30 * 24 * 3600;
pub const MAX_INVITE_USES: i32 = 10_000;
pub const MAX_EVENT_DESCRIPTION_CHARS: usize = 3000;
pub const MAX_PLACE_NAME_CHARS: usize = 100;
pub const MAX_PLACE_ADDRESS_CHARS: usize = 200;
pub const MAX_EVENT_CAPACITY: i32 = 100_000;
pub const MAX_URL_BYTES: usize = 500;
pub const MAX_SLOW_MODE_SECONDS: i32 = 6 * 3600;
pub const MIN_TIMEOUT_SECONDS: i32 = 60;
pub const MAX_TIMEOUT_SECONDS: i32 = 28 * 24 * 3600;

pub fn community_name(input: &str) -> AppResult<String> {
    let name = text::clean_line(input);
    ensure!(
        !name.is_empty() && text::fits(&name, MAX_NAME_CHARS),
        invalid,
        "NAME_INVALID",
        "a community name has 1-{MAX_NAME_CHARS} characters"
    );
    Ok(name)
}

/// A community handle: the format and reserved words of user handles, in a
/// namespace of its own.
pub fn handle(input: &str) -> AppResult<String> {
    tokuly_identity::users::validate_handle(input)
}

pub fn description(input: &str, max_chars: usize) -> AppResult<String> {
    let description = text::clean(input);
    ensure!(
        text::fits(&description, max_chars),
        invalid,
        "DESCRIPTION_TOO_LONG",
        "the description has at most {max_chars} characters"
    );
    Ok(description)
}

pub fn category(value: EnumValue<CommunityCategory>) -> AppResult<i32> {
    match value.as_known() {
        Some(category) if category != CommunityCategory::COMMUNITY_CATEGORY_UNSPECIFIED => Ok(category as i32),
        _ => Err(AppError::invalid("CATEGORY_INVALID", "choose a category")),
    }
}

pub fn visibility(value: EnumValue<CommunityVisibility>) -> AppResult<i32> {
    match value.as_known() {
        Some(visibility) if visibility != CommunityVisibility::COMMUNITY_VISIBILITY_UNSPECIFIED => {
            Ok(visibility as i32)
        }
        _ => Err(AppError::invalid("VISIBILITY_INVALID", "choose public, restricted or hidden")),
    }
}

/// A locale tag such as `ja` or `en-US`.
pub fn language(input: &str) -> AppResult<String> {
    let language = input.trim();
    let ok = (2..=16).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    ensure!(ok, invalid, "LANGUAGE_INVALID", "the language is a locale tag such as ja or en");
    Ok(language.to_owned())
}

/// Tags as stored: normalized, each once, in the order given.
pub fn tags(inputs: &[String], max: usize) -> AppResult<Vec<String>> {
    let mut tags: Vec<String> = Vec::with_capacity(inputs.len());
    for input in inputs {
        let tag = tag(input)?;
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    ensure!(tags.len() <= max, invalid, "TOO_MANY_TAGS", "at most {max} tags");
    Ok(tags)
}

/// One tag (also used for filters).
pub fn tag(input: &str) -> AppResult<String> {
    text::normalize_tag(input, MAX_TAG_CHARS).ok_or_else(|| {
        AppError::invalid("TAG_INVALID", format!("tags have 1-{MAX_TAG_CHARS} letters, digits or underscores"))
    })
}

pub fn rules(inputs: Vec<pb::Rule>) -> AppResult<Vec<pb::Rule>> {
    ensure!(inputs.len() <= MAX_RULES, invalid, "TOO_MANY_RULES", "at most {MAX_RULES} rules");
    inputs
        .into_iter()
        .map(|rule| {
            let title = text::clean_line(&rule.title);
            let body = text::clean(&rule.body);
            ensure!(
                !title.is_empty() && text::fits(&title, MAX_RULE_TITLE_CHARS) && text::fits(&body, MAX_RULE_BODY_CHARS),
                invalid,
                "RULE_INVALID",
                "a rule has a title of 1-{MAX_RULE_TITLE_CHARS} characters and a body of at most {MAX_RULE_BODY_CHARS}"
            );
            Ok(pb::Rule { title, body, ..Default::default() })
        })
        .collect()
}

pub fn join_questions(inputs: &[String]) -> AppResult<Vec<String>> {
    ensure!(inputs.len() <= MAX_QUESTIONS, invalid, "TOO_MANY_QUESTIONS", "at most {MAX_QUESTIONS} join questions");
    inputs
        .iter()
        .map(|input| {
            let question = text::clean_line(input);
            ensure!(
                !question.is_empty() && text::fits(&question, MAX_QUESTION_CHARS),
                invalid,
                "QUESTION_INVALID",
                "a join question has 1-{MAX_QUESTION_CHARS} characters"
            );
            Ok(question)
        })
        .collect()
}

/// Answers to a community's join questions: one per question, none empty.
pub fn join_answers(questions: &[String], answers: &[String]) -> AppResult<Vec<pb::JoinAnswer>> {
    let cleaned: Vec<String> = answers.iter().map(|answer| text::clean(answer)).collect();
    ensure!(
        cleaned.len() == questions.len() && cleaned.iter().all(|answer| !answer.is_empty()),
        invalid,
        "ANSWERS_REQUIRED",
        "answer each of the community's {} join questions",
        questions.len()
    );
    ensure!(
        cleaned.iter().all(|answer| text::fits(answer, MAX_ANSWER_CHARS)),
        invalid,
        "ANSWER_TOO_LONG",
        "an answer has at most {MAX_ANSWER_CHARS} characters"
    );
    Ok(questions
        .iter()
        .zip(cleaned)
        .map(|(question, answer)| pb::JoinAnswer { question: question.clone(), answer, ..Default::default() })
        .collect())
}

pub fn school_name(input: &str) -> AppResult<String> {
    tokuly_identity::users::validate_school_name(input)
}

pub fn notification_level(value: EnumValue<NotificationLevel>) -> AppResult<i32> {
    match value.as_known() {
        Some(level) if level != NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED => Ok(level as i32),
        _ => Err(AppError::invalid("LEVEL_INVALID", "choose all, mentions or none")),
    }
}

pub fn channel_name(input: &str) -> AppResult<String> {
    let name = text::clean_line(input);
    ensure!(
        !name.is_empty() && text::fits(&name, MAX_CHANNEL_NAME_CHARS),
        invalid,
        "CHANNEL_NAME_INVALID",
        "a channel name has 1-{MAX_CHANNEL_NAME_CHARS} characters"
    );
    Ok(name)
}

/// The emoji of a channel: empty, or one character that is not a letter or digit.
pub fn emoji(input: &str) -> AppResult<String> {
    use unicode_segmentation::UnicodeSegmentation as _;
    let emoji = input.trim();
    let mut graphemes = emoji.graphemes(true);
    let ok = match (graphemes.next(), graphemes.next()) {
        (None, _) => true,
        (Some(one), None) => one.len() <= 64 && !one.chars().any(|c| c.is_alphanumeric() || c.is_control()),
        _ => false,
    };
    ensure!(ok, invalid, "EMOJI_INVALID", "the channel emoji is a single emoji");
    Ok(emoji.to_owned())
}

/// Unspecified reads as "everyone".
pub fn posting(value: EnumValue<PostingPolicy>) -> AppResult<i32> {
    match value.as_known() {
        Some(PostingPolicy::POSTING_POLICY_UNSPECIFIED | PostingPolicy::POSTING_POLICY_EVERYONE) => {
            Ok(PostingPolicy::POSTING_POLICY_EVERYONE as i32)
        }
        Some(PostingPolicy::POSTING_POLICY_MODERATORS) => Ok(PostingPolicy::POSTING_POLICY_MODERATORS as i32),
        None => Err(AppError::invalid("POSTING_INVALID", "choose who may post: everyone or moderators")),
    }
}

/// The title of a thread or an event.
pub fn title(input: &str) -> AppResult<String> {
    let title = text::clean_line(input);
    ensure!(
        !title.is_empty() && text::fits(&title, MAX_TITLE_CHARS),
        invalid,
        "TITLE_INVALID",
        "a title has 1-{MAX_TITLE_CHARS} characters"
    );
    Ok(title)
}

pub fn body(input: &str, max_chars: usize) -> AppResult<String> {
    let body = text::clean(input);
    ensure!(text::fits(&body, max_chars), invalid, "BODY_TOO_LONG", "the text has at most {max_chars} characters");
    Ok(body)
}

/// A reason given by the staff. `required` turns an empty one into `REASON_REQUIRED`.
pub fn reason(input: &str, required: bool) -> AppResult<String> {
    let reason = text::clean_line(input);
    ensure!(!required || !reason.is_empty(), invalid, "REASON_REQUIRED", "give a reason");
    ensure!(
        text::fits(&reason, MAX_REASON_CHARS),
        invalid,
        "REASON_TOO_LONG",
        "a reason has at most {MAX_REASON_CHARS} characters"
    );
    Ok(reason)
}

pub fn query(input: &str) -> AppResult<String> {
    let query = text::clean_line(input);
    ensure!(text::fits(&query, MAX_QUERY_CHARS), invalid, "QUERY_TOO_LONG", "the search text is too long");
    Ok(query)
}

/// A poll as it will be stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NewPoll {
    pub options: Vec<String>,
    pub multiple: bool,
    pub closes_at: Option<Time>,
}

/// Shortest and longest time a poll may run when it has a deadline.
const MIN_POLL_RUN: Duration = Duration::minutes(5);
const MAX_POLL_RUN: Duration = Duration::days(30);

pub fn poll(input: &pb::PollInput, now: Time) -> AppResult<NewPoll> {
    let invalid = |message: &str| AppError::invalid("POLL_INVALID", message.to_owned());
    let options: Vec<String> = input.options.iter().map(|option| text::clean_line(option)).collect();
    if !(2..=MAX_POLL_OPTIONS).contains(&options.len()) {
        return Err(invalid("a poll has 2-6 options"));
    }
    if options.iter().any(|option| option.is_empty() || !text::fits(option, MAX_POLL_OPTION_CHARS)) {
        return Err(invalid("a poll option has 1-80 characters"));
    }
    let folded: Vec<String> = options.iter().map(|option| text::fold(option)).collect();
    if folded.iter().enumerate().any(|(i, option)| folded[..i].contains(option)) {
        return Err(invalid("poll options must differ"));
    }
    let closes_at = input.closes_at.as_option().map(from_ts);
    if let Some(closes_at) = closes_at
        && !(now + MIN_POLL_RUN..=now + MAX_POLL_RUN).contains(&closes_at)
    {
        return Err(invalid("a poll closes between 5 minutes and 30 days from now"));
    }
    Ok(NewPoll { options, multiple: input.multiple_choice, closes_at })
}

/// Muted words as stored: folded (see [`text::fold`]), each once.
pub fn muted_words(inputs: &[String]) -> AppResult<Vec<String>> {
    let mut words: Vec<String> = Vec::with_capacity(inputs.len());
    for input in inputs {
        let word = text::fold(&text::clean_line(input));
        ensure!(
            !word.is_empty() && text::fits(&word, MAX_MUTED_WORD_CHARS),
            invalid,
            "MUTED_WORD_INVALID",
            "a muted word has 1-{MAX_MUTED_WORD_CHARS} characters"
        );
        if !words.contains(&word) {
            words.push(word);
        }
    }
    ensure!(words.len() <= MAX_MUTED_WORDS, invalid, "TOO_MANY_MUTED_WORDS", "at most {MAX_MUTED_WORDS} muted words");
    Ok(words)
}

/// The community's own auto-hide threshold: 0 (the server's default) or 2-50.
pub fn auto_hide_reports(value: i32) -> AppResult<i32> {
    ensure!(
        value == 0 || (2..=50).contains(&value),
        invalid,
        "THRESHOLD_INVALID",
        "the auto-hide threshold is 0 (default) or between 2 and 50 reports"
    );
    Ok(value)
}

/// Where an in-person event takes place.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub address: String,
    pub location: Option<(f64, f64)>,
}

pub fn place(input: &pb::EventPlace) -> AppResult<Place> {
    let name = text::clean_line(&input.name);
    let address = text::clean_line(&input.address);
    ensure!(
        !name.is_empty() && text::fits(&name, MAX_PLACE_NAME_CHARS) && text::fits(&address, MAX_PLACE_ADDRESS_CHARS),
        invalid,
        "PLACE_INVALID",
        "a place has a name of 1-{MAX_PLACE_NAME_CHARS} characters"
    );
    let location = match input.location.as_option() {
        Some(point) => {
            let ok = point.lat.is_finite()
                && point.lng.is_finite()
                && (-90.0..=90.0).contains(&point.lat)
                && (-180.0..=180.0).contains(&point.lng);
            ensure!(ok, invalid, "PLACE_INVALID", "the place's coordinates are out of range");
            Some((point.lat, point.lng))
        }
        None => None,
    };
    Ok(Place { name, address, location })
}

/// The link of an online event: empty, or an http(s) URL.
pub fn online_url(input: &str) -> AppResult<String> {
    let url = input.trim();
    if url.is_empty() {
        return Ok(String::new());
    }
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    let ok = url.len() <= MAX_URL_BYTES
        && rest.is_some_and(|rest| {
            let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
            !host.is_empty() && !url.chars().any(|c| c.is_whitespace() || c.is_control())
        });
    ensure!(ok, invalid, "URL_INVALID", "the link must be an http(s) URL");
    Ok(url.to_owned())
}

pub fn capacity(value: i32) -> AppResult<i32> {
    ensure!(
        (0..=MAX_EVENT_CAPACITY).contains(&value),
        invalid,
        "CAPACITY_INVALID",
        "the capacity is between 0 (unlimited) and {MAX_EVENT_CAPACITY}"
    );
    Ok(value)
}

/// How long an event may last, and how long one lasts when no end is given.
const MAX_EVENT_LENGTH: Duration = Duration::days(31);
const DEFAULT_EVENT_LENGTH: Duration = Duration::hours(2);

/// Start and end of an event. `must_be_future` is off when an edit leaves
/// the start where it was.
pub fn event_times(starts_at: Time, ends_at: Option<Time>, now: Time, must_be_future: bool) -> AppResult<(Time, Time)> {
    let ends_at = ends_at.unwrap_or(starts_at + DEFAULT_EVENT_LENGTH);
    ensure!(!must_be_future || starts_at > now, invalid, "TIME_INVALID", "an event starts in the future");
    ensure!(ends_at > starts_at, invalid, "TIME_INVALID", "an event ends after it starts");
    ensure!(ends_at - starts_at <= MAX_EVENT_LENGTH, invalid, "TIME_INVALID", "an event lasts at most 31 days");
    Ok((starts_at, ends_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason_of<T: std::fmt::Debug>(result: AppResult<T>) -> String {
        result.unwrap_err().reason.into_owned()
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(community_name("  3年B組\n").unwrap(), "3年B組");
        assert_eq!(reason_of(community_name("   ")), "NAME_INVALID");
        assert_eq!(reason_of(community_name(&"あ".repeat(51))), "NAME_INVALID");
        assert!(community_name(&"あ".repeat(50)).is_ok());
    }

    #[test]
    fn handles_follow_the_user_handle_rules() {
        assert_eq!(handle("Apex_Club").unwrap(), "apex_club");
        assert_eq!(reason_of(handle("ab")), "HANDLE_INVALID");
        assert_eq!(reason_of(handle("admin")), "HANDLE_RESERVED");
    }

    #[test]
    fn enums_must_be_chosen() {
        assert_eq!(reason_of(category(EnumValue::from(0))), "CATEGORY_INVALID");
        assert_eq!(reason_of(category(EnumValue::from(99))), "CATEGORY_INVALID");
        assert_eq!(category(CommunityCategory::COMMUNITY_CATEGORY_GAME.into()).unwrap(), 3);
        assert_eq!(reason_of(visibility(EnumValue::from(0))), "VISIBILITY_INVALID");
        assert_eq!(visibility(CommunityVisibility::COMMUNITY_VISIBILITY_HIDDEN.into()).unwrap(), 3);
        assert_eq!(reason_of(notification_level(EnumValue::from(0))), "LEVEL_INVALID");
        assert_eq!(posting(EnumValue::from(0)).unwrap(), 1);
        assert_eq!(reason_of(posting(EnumValue::from(9))), "POSTING_INVALID");
    }

    #[test]
    fn tags_are_normalized_deduplicated_and_capped() {
        let input: Vec<String> = ["#APEX", "apex", "ランク"].iter().map(|s| s.to_string()).collect();
        assert_eq!(tags(&input, MAX_TAGS).unwrap(), vec!["apex", "ランク"]);
        assert_eq!(reason_of(tags(&["a b".to_owned()], MAX_TAGS)), "TAG_INVALID");
        let many: Vec<String> = (0..11).map(|i| format!("tag{i}")).collect();
        assert_eq!(reason_of(tags(&many, MAX_TAGS)), "TOO_MANY_TAGS");
    }

    #[test]
    fn rules_and_questions_are_checked() {
        let rule = |title: &str, body: &str| pb::Rule { title: title.into(), body: body.into(), ..Default::default() };
        assert_eq!(rules(vec![rule(" なかよく ", " けんかしない ")]).unwrap()[0].title, "なかよく");
        assert_eq!(reason_of(rules(vec![rule("", "x")])), "RULE_INVALID");
        assert_eq!(reason_of(rules(vec![rule("t", "x"); 21])), "TOO_MANY_RULES");
        assert_eq!(reason_of(join_questions(&[String::new()])), "QUESTION_INVALID");
        assert_eq!(reason_of(join_questions(&vec!["q".to_owned(); 6])), "TOO_MANY_QUESTIONS");
    }

    #[test]
    fn every_join_question_needs_an_answer() {
        let questions = vec!["何年生?".to_owned(), "なぜ入りたい?".to_owned()];
        let answers = join_answers(&questions, &["2年".to_owned(), " 楽しそう ".to_owned()]).unwrap();
        assert_eq!(answers[1].question, "なぜ入りたい?");
        assert_eq!(answers[1].answer, "楽しそう");
        assert_eq!(reason_of(join_answers(&questions, &["2年".to_owned()])), "ANSWERS_REQUIRED");
        assert_eq!(reason_of(join_answers(&questions, &["2年".to_owned(), "  ".to_owned()])), "ANSWERS_REQUIRED");
        assert_eq!(reason_of(join_answers(&questions, &["2年".to_owned(), "あ".repeat(501)])), "ANSWER_TOO_LONG");
        assert!(join_answers(&[], &[]).unwrap().is_empty());
    }

    #[test]
    fn channel_emoji_is_one_emoji_or_nothing() {
        assert_eq!(emoji(" 🎮 ").unwrap(), "🎮");
        assert_eq!(emoji("").unwrap(), "");
        assert_eq!(emoji("👨‍👩‍👧").unwrap(), "👨‍👩‍👧");
        assert_eq!(reason_of(emoji("a")), "EMOJI_INVALID");
        assert_eq!(reason_of(emoji("🎮🎮")), "EMOJI_INVALID");
    }

    #[test]
    fn reasons_can_be_required() {
        assert_eq!(reason(" spam ", true).unwrap(), "spam");
        assert_eq!(reason("", false).unwrap(), "");
        assert_eq!(reason_of(reason("  ", true)), "REASON_REQUIRED");
        assert_eq!(reason_of(reason(&"あ".repeat(301), false)), "REASON_TOO_LONG");
    }

    #[test]
    fn polls_need_two_to_six_distinct_options_and_a_sane_deadline() {
        let now = chrono::Utc::now();
        let input = |options: &[&str], closes_in: Option<Duration>| pb::PollInput {
            options: options.iter().map(|o| o.to_string()).collect(),
            multiple_choice: true,
            closes_at: opt_ts(closes_in.map(|d| now + d)),
            ..Default::default()
        };
        let ok = poll(&input(&[" はい ", "いいえ"], Some(Duration::hours(1))), now).unwrap();
        assert_eq!(ok.options, vec!["はい", "いいえ"]);
        assert!(ok.multiple && ok.closes_at.is_some());
        assert!(poll(&input(&["a", "b"], None), now).unwrap().closes_at.is_none());
        for bad in [
            input(&["only"], None),
            input(&["1", "2", "3", "4", "5", "6", "7"], None),
            input(&["a", ""], None),
            input(&["ＡＢＣ", "abc"], None),
            input(&["a", "b"], Some(Duration::minutes(1))),
            input(&["a", "b"], Some(Duration::days(31))),
        ] {
            assert_eq!(reason_of(poll(&bad, now)), "POLL_INVALID", "{bad:?}");
        }
    }

    #[test]
    fn muted_words_are_stored_folded() {
        let words = muted_words(&["バカ".to_owned(), "ばか".to_owned(), " Spam ".to_owned()]).unwrap();
        assert_eq!(words, vec!["ばか", "spam"]);
        assert_eq!(reason_of(muted_words(&[" ".to_owned()])), "MUTED_WORD_INVALID");
        let many: Vec<String> = (0..201).map(|i| format!("w{i}")).collect();
        assert_eq!(reason_of(muted_words(&many)), "TOO_MANY_MUTED_WORDS");
        assert_eq!(reason_of(auto_hide_reports(1)), "THRESHOLD_INVALID");
        assert_eq!(auto_hide_reports(0).unwrap(), 0);
        assert_eq!(auto_hide_reports(5).unwrap(), 5);
    }

    #[test]
    fn places_and_links_are_checked() {
        let at = |lat: f64, lng: f64| pb::EventPlace {
            name: "第2体育館".into(),
            location: tokuly_proto::common::LatLng { lat, lng, ..Default::default() }.into(),
            ..Default::default()
        };
        assert_eq!(place(&at(35.6, 139.7)).unwrap().location, Some((35.6, 139.7)));
        assert_eq!(reason_of(place(&at(91.0, 0.0))), "PLACE_INVALID");
        assert_eq!(reason_of(place(&pb::EventPlace::default())), "PLACE_INVALID");
        assert_eq!(online_url(" https://meet.example.com/abc ").unwrap(), "https://meet.example.com/abc");
        assert_eq!(online_url("").unwrap(), "");
        for bad in ["javascript:alert(1)", "https://", "ftp://example.com", "https://exa mple.com"] {
            assert_eq!(reason_of(online_url(bad)), "URL_INVALID", "{bad}");
        }
        assert_eq!(reason_of(capacity(-1)), "CAPACITY_INVALID");
    }

    #[test]
    fn event_times_are_ordered_and_bounded() {
        let now = chrono::Utc::now();
        let start = now + Duration::days(1);
        assert_eq!(event_times(start, None, now, true).unwrap().1, start + Duration::hours(2));
        assert_eq!(reason_of(event_times(now - Duration::hours(1), None, now, true)), "TIME_INVALID");
        assert!(event_times(now - Duration::hours(1), None, now, false).is_ok());
        assert_eq!(reason_of(event_times(start, Some(start), now, true)), "TIME_INVALID");
        assert_eq!(reason_of(event_times(start, Some(start + Duration::days(32)), now, true)), "TIME_INVALID");
    }
}
