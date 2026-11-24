//! What a note says, checked the same way wherever it is written: posting,
//! editing and saving a draft. Pure functions, no I/O.
use tokuly_core::prelude::*;
use tokuly_proto::{
    media::Attachment,
    notes::{AttachmentDetails, AttachmentDetailsInput, PollInput, PollResultsVisibility, RichText},
};

use crate::{
    markup::{self, Facts},
    text,
};

/// Longest note, in characters. Long enough for a club announcement or a
/// short story told in one go (Misskey allows the same); the composer still
/// opens as a one-line 「今何してる?」 field and only shows the count near the
/// end, so a quick note stays quick.
pub const MAX_NOTE_CHARS: usize = 3000;
/// Longest content warning line.
pub const MAX_CW_CHARS: usize = 100;
/// Longest alt text of one attachment.
pub const MAX_ALT_TEXT_CHARS: usize = 1500;
/// Poll limits.
pub const MIN_POLL_CHOICES: usize = 2;
pub const MAX_POLL_CHOICES: usize = 10;
pub const MAX_POLL_CHOICE_CHARS: usize = 50;
pub const MIN_POLL_SECONDS: i64 = 5 * 60;
pub const MAX_POLL_SECONDS: i64 = 30 * 24 * 3600;

/// A note's text, parsed, and its content warning.
#[derive(Debug, Clone, Default)]
pub(crate) struct Content {
    /// The text as stored (cleaned).
    pub text: String,
    pub body: RichText,
    pub facts: Facts,
    pub language: &'static str,
    pub content_warning: bool,
    pub content_warning_text: String,
}

/// Checks and parses a note's text and content warning. Setting a warning
/// line folds the note even without the flag.
pub(crate) fn prepare(text_input: &str, content_warning: bool, warning_input: &str) -> AppResult<Content> {
    let cleaned = text::clean(text_input);
    ensure!(
        text::fits(&cleaned, MAX_NOTE_CHARS),
        invalid,
        "NOTE_TOO_LONG",
        "a note has at most {MAX_NOTE_CHARS} characters"
    );
    let warning = one_line(warning_input);
    ensure!(
        text::fits(&warning, MAX_CW_CHARS),
        invalid,
        "CW_TOO_LONG",
        "a content warning has at most {MAX_CW_CHARS} characters"
    );
    let body = markup::parse(&cleaned);
    let facts = markup::facts(&body);
    Ok(Content {
        language: text::language(&cleaned),
        text: cleaned,
        body,
        facts,
        content_warning: content_warning || !warning.is_empty(),
        content_warning_text: warning,
    })
}

fn one_line(input: &str) -> String {
    text::clean(input).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The details of each attachment, in the attachments' order, from what the
/// author said about them by asset ID (anything about other assets is ignored).
pub(crate) fn attachment_details(
    attachments: &[Attachment],
    inputs: &[AttachmentDetailsInput],
) -> AppResult<Vec<AttachmentDetails>> {
    attachments
        .iter()
        .map(|attachment| {
            let input = inputs.iter().find(|input| input.asset_id == attachment.asset_id);
            let alt_text = input.map(|input| text::clean(&input.alt_text)).unwrap_or_default();
            ensure!(
                text::fits(&alt_text, MAX_ALT_TEXT_CHARS),
                invalid,
                "ALT_TEXT_TOO_LONG",
                "alt text has at most {MAX_ALT_TEXT_CHARS} characters"
            );
            Ok(AttachmentDetails {
                asset_id: attachment.asset_id.clone(),
                alt_text,
                sensitive: input.is_some_and(|input| input.sensitive),
                ..Default::default()
            })
        })
        .collect()
}

/// A poll as it will be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewPoll {
    pub choices: Vec<String>,
    pub multiple: bool,
    pub duration_seconds: i64,
    /// `PollResultsVisibility` as stored (1–3).
    pub results_visibility: i32,
}

/// Checks a poll: 2–10 different choices of 1–50 characters, 5 minutes to
/// 30 days. `None` when the note has no poll.
pub(crate) fn poll(input: Option<&PollInput>) -> AppResult<Option<NewPoll>> {
    let Some(input) = input else { return Ok(None) };
    let invalid = |message: &'static str| AppError::invalid("INVALID_POLL", message);
    let choices: Vec<String> = input.choices.iter().map(|choice| one_line(choice)).collect();
    if !(MIN_POLL_CHOICES..=MAX_POLL_CHOICES).contains(&choices.len()) {
        return Err(invalid("a poll has 2 to 10 choices"));
    }
    if choices.iter().any(|choice| choice.is_empty() || !text::fits(choice, MAX_POLL_CHOICE_CHARS)) {
        return Err(invalid("a choice has 1 to 50 characters"));
    }
    let mut seen = choices.clone();
    seen.sort();
    seen.dedup();
    if seen.len() != choices.len() {
        return Err(invalid("the choices are all different"));
    }
    if !(MIN_POLL_SECONDS..=MAX_POLL_SECONDS).contains(&input.duration_seconds) {
        return Err(invalid("a poll runs from 5 minutes to 30 days"));
    }
    let results_visibility = match input.results_visibility.as_known() {
        Some(PollResultsVisibility::POLL_RESULTS_VISIBILITY_UNSPECIFIED)
        | Some(PollResultsVisibility::POLL_RESULTS_VISIBILITY_ALWAYS) => 1,
        Some(PollResultsVisibility::POLL_RESULTS_VISIBILITY_AFTER_VOTING) => 2,
        Some(PollResultsVisibility::POLL_RESULTS_VISIBILITY_AFTER_END) => 3,
        None => return Err(invalid("unknown results visibility")),
    };
    Ok(Some(NewPoll {
        choices,
        multiple: input.multiple,
        duration_seconds: input.duration_seconds,
        results_visibility,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_warning_are_checked_and_parsed() {
        let content = prepare("  **やった** #春\r\n", false, "").unwrap();
        assert_eq!(content.text, "**やった** #春");
        assert_eq!(content.facts.hashtags, vec!["春"]);
        assert_eq!(content.language, "ja");
        assert!(!content.content_warning);
        let warned = prepare("ネタバレ", false, "  映画の\n結末  ").unwrap();
        assert!(warned.content_warning);
        assert_eq!(warned.content_warning_text, "映画の 結末");
        assert!(prepare(&"あ".repeat(MAX_NOTE_CHARS), false, "").is_ok());
        assert_eq!(prepare(&"あ".repeat(MAX_NOTE_CHARS + 1), false, "").unwrap_err().reason, "NOTE_TOO_LONG");
        assert_eq!(prepare("x", true, &"注".repeat(MAX_CW_CHARS + 1)).unwrap_err().reason, "CW_TOO_LONG");
    }

    #[test]
    fn details_follow_the_attachments() {
        let attachment = |id: &str| Attachment { asset_id: id.into(), ..Default::default() };
        let input = |id: &str, alt: &str, sensitive| AttachmentDetailsInput {
            asset_id: id.into(),
            alt_text: alt.into(),
            sensitive,
            ..Default::default()
        };
        let details = attachment_details(
            &[attachment("a"), attachment("b")],
            &[input("b", " 夕焼け ", true), input("zzz", "ignored", true)],
        )
        .unwrap();
        assert_eq!(
            details.iter().map(|d| (d.asset_id.as_str(), d.alt_text.as_str(), d.sensitive)).collect::<Vec<_>>(),
            vec![("a", "", false), ("b", "夕焼け", true)]
        );
        let long = "a".repeat(MAX_ALT_TEXT_CHARS + 1);
        assert_eq!(
            attachment_details(&[attachment("a")], &[input("a", &long, false)]).unwrap_err().reason,
            "ALT_TEXT_TOO_LONG"
        );
    }

    #[test]
    fn polls_are_checked() {
        let poll_of = |choices: &[&str], seconds: i64| PollInput {
            choices: choices.iter().map(|c| c.to_string()).collect(),
            duration_seconds: seconds,
            ..Default::default()
        };
        let ok = poll(Some(&poll_of(&["いぬ", "ねこ"], 3600))).unwrap().unwrap();
        assert_eq!((ok.choices.len(), ok.results_visibility), (2, 1));
        assert_eq!(poll(None).unwrap(), None);
        for bad in [
            poll_of(&["one"], 3600),
            poll_of(&["a"; 11], 3600),
            poll_of(&["a", "a"], 3600),
            poll_of(&["a", " "], 3600),
            poll_of(&["a", "b"], 60),
            poll_of(&["a", "b"], MAX_POLL_SECONDS + 1),
            poll_of(&["a", &"b".repeat(MAX_POLL_CHOICE_CHARS + 1)], 3600),
        ] {
            assert_eq!(poll(Some(&bad)).unwrap_err().reason, "INVALID_POLL", "{bad:?}");
        }
    }
}
