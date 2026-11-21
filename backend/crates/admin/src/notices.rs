//! Everything the admin domain says to users: notification requests and the
//! suspension emails, in Japanese and English.
//!
//! Suspended users are signed out and cannot read their inbox, so suspensions
//! (and their lifting) are announced by email instead of a notification.
use chrono::{DateTime, Utc};
use tokuly_core::outbound::{Localized, NotificationRequest};
use uuid::Uuid;

pub(crate) const KIND_WARNING: &str = "system.warning";
pub(crate) const KIND_CONTENT_REMOVED: &str = "system.content_removed";
pub(crate) const KIND_REPORT_RESOLVED: &str = "system.report_resolved";
/// Not a `system.` kind on purpose: notify files `announcement.*` under the
/// "feed" category, which users can switch off and which does not wake a
/// sleeping device. Product news should not be as loud as a warning.
pub(crate) const KIND_ANNOUNCEMENT: &str = "announcement.published";

/// Where system notices lead: they have no page of their own.
const INBOX_PATH: &str = "/notifications";

/// Longest content title quoted inside a notification.
const QUOTED_TITLE_CHARS: usize = 40;

/// Shortens `text` to `max` characters, marking the cut with an ellipsis.
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// "You were warned" — inbox and push.
pub(crate) fn warning(user_id: Uuid, reason: &str) -> NotificationRequest {
    NotificationRequest::new(
        user_id,
        KIND_WARNING,
        Localized::new("運営からの警告です", "A warning from the Tokuly team"),
        Localized::new(
            format!("コミュニティルールに反する行為が確認されました。理由: {reason}"),
            format!("Something on your account goes against the community rules. Reason: {reason}"),
        ),
        INBOX_PATH,
    )
}

/// "Your content was taken down" — inbox and push. `title` is the content's
/// title or excerpt and may be empty.
pub(crate) fn content_removed(user_id: Uuid, subject: (i32, Uuid), title: &str, reason: &str) -> NotificationRequest {
    let quoted = truncate_chars(title.trim(), QUOTED_TITLE_CHARS);
    let body = if quoted.is_empty() {
        Localized::new(
            format!("あなたの投稿はコミュニティルールに違反していたため削除されました。理由: {reason}"),
            format!("Your content was removed for breaking the community rules. Reason: {reason}"),
        )
    } else {
        Localized::new(
            format!("「{quoted}」はコミュニティルールに違反していたため削除されました。理由: {reason}"),
            format!("\"{quoted}\" was removed for breaking the community rules. Reason: {reason}"),
        )
    };
    let mut request =
        NotificationRequest::new(user_id, KIND_CONTENT_REMOVED, Localized::new("投稿が削除されました", "Your content was removed"), body, INBOX_PATH);
    // Set directly: the type arrives as a number from another domain and may
    // be newer than the enum this crate was built with.
    request.subject = Some(subject);
    request
}

/// "We handled your report" — inbox only, it is not urgent.
pub(crate) fn report_resolved(reporter_id: Uuid, case_id: Uuid, action_taken: bool) -> NotificationRequest {
    let body = if action_taken {
        Localized::new(
            "通報いただいた内容を確認し、対応しました。Tokulyを安心して楽しめる場所にするご協力に感謝します!",
            "We reviewed what you reported and took action. Thanks for helping keep Tokuly safe!",
        )
    } else {
        Localized::new(
            "通報いただいた内容を確認しましたが、今回はルール違反にはあたらないと判断しました。",
            "We reviewed what you reported and found that it does not break our rules this time.",
        )
    };
    NotificationRequest::new(reporter_id, KIND_REPORT_RESOLVED, Localized::new("通報ありがとうございます!", "Thanks for your report!"), body, INBOX_PATH)
        .inbox_only()
        .collapse(format!("report-resolved:{case_id}"))
        .data(serde_json::json!({ "case_id": case_id, "action_taken": action_taken }))
}

/// "There is news" — sent to every active user when an announcement is
/// published with the notify option.
pub(crate) fn announcement(user_id: Uuid, announcement_id: Uuid, title: Localized, summary: Localized, image_url: Option<&str>) -> NotificationRequest {
    let request = NotificationRequest::new(user_id, KIND_ANNOUNCEMENT, title, summary, format!("/news/{announcement_id}"))
        .collapse(format!("announcement:{announcement_id}"))
        .data(serde_json::json!({ "announcement_id": announcement_id }));
    match image_url {
        Some(url) => request.image(url),
        None => request,
    }
}

/// An email as `core.mail.send` wants it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mail {
    pub subject: String,
    pub text: String,
    pub html: String,
}

impl Mail {
    fn from_paragraphs(subject: &str, paragraphs: &[String]) -> Self {
        let html = paragraphs.iter().map(|p| format!("<p>{}</p>", escape_html(p).replace('\n', "<br>"))).collect::<String>();
        Self { subject: subject.to_owned(), text: paragraphs.join("\n\n"), html }
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn is_japanese(locale: &str) -> bool {
    locale.to_ascii_lowercase().starts_with("ja")
}

/// `expires_at` in the user's time zone, e.g. `2026-12-01 09:00 (Asia/Tokyo)`.
fn local_time(at: DateTime<Utc>, time_zone: &str) -> String {
    match time_zone.parse::<chrono_tz::Tz>() {
        Ok(tz) => format!("{} ({time_zone})", at.with_timezone(&tz).format("%Y-%m-%d %H:%M")),
        Err(_) => format!("{} (UTC)", at.format("%Y-%m-%d %H:%M")),
    }
}

/// The email telling a user their account was suspended or banned.
pub(crate) fn suspension_mail(locale: &str, time_zone: &str, reason: &str, expires_at: Option<DateTime<Utc>>) -> Mail {
    if is_japanese(locale) {
        let period = match expires_at {
            Some(at) => format!("停止期間: {} まで", local_time(at, time_zone)),
            None => "停止期間: 無期限".to_owned(),
        };
        Mail::from_paragraphs(
            "【Tokuly】アカウントが停止されました",
            &[
                "いつもTokulyをご利用いただきありがとうございます。".to_owned(),
                "コミュニティルールに違反する行為が確認されたため、あなたのアカウントを停止しました。停止中はログインできません。".to_owned(),
                format!("理由: {reason}\n{period}"),
                "お心当たりがない場合は、このメールに返信してお知らせください。".to_owned(),
            ],
        )
    } else {
        let period = match expires_at {
            Some(at) => format!("Suspended until: {}", local_time(at, time_zone)),
            None => "Suspended: permanently".to_owned(),
        };
        Mail::from_paragraphs(
            "Your Tokuly account has been suspended",
            &[
                "Thank you for using Tokuly.".to_owned(),
                "We found activity on your account that breaks the community rules, so the account has been suspended. You cannot sign in while it is suspended.".to_owned(),
                format!("Reason: {reason}\n{period}"),
                "If you believe this is a mistake, reply to this email and let us know.".to_owned(),
            ],
        )
    }
}

/// The email telling a user they can sign in again.
pub(crate) fn suspension_lifted_mail(locale: &str) -> Mail {
    if is_japanese(locale) {
        Mail::from_paragraphs(
            "【Tokuly】アカウントの停止が解除されました",
            &[
                "アカウントの停止が解除されました。またTokulyにログインできます!".to_owned(),
                "これからもコミュニティルールを守って、Tokulyを楽しんでくださいね。".to_owned(),
            ],
        )
    } else {
        Mail::from_paragraphs(
            "Your Tokuly account is active again",
            &[
                "The suspension of your account has been lifted. You can sign in to Tokuly again!".to_owned(),
                "Please keep the community rules in mind, and enjoy Tokuly.".to_owned(),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::common::SubjectType;

    use super::*;

    #[test]
    fn truncation_counts_characters_and_marks_the_cut() {
        assert_eq!(truncate_chars("短い", 5), "短い");
        assert_eq!(truncate_chars("あいうえおかきくけこ", 5), "あいうえ…");
        assert_eq!(truncate_chars("abcdef", 6), "abcdef");
    }

    #[test]
    fn warning_goes_to_inbox_and_push_with_the_reason() {
        let user = Uuid::now_v7();
        let n = warning(user, "スパム投稿");
        assert_eq!((n.user_id, n.kind.as_str()), (user, KIND_WARNING));
        assert!(n.inbox && n.push && n.actor_id.is_none());
        assert!(n.body.ja.contains("スパム投稿") && n.body.en.contains("スパム投稿"));
    }

    #[test]
    fn content_removed_quotes_a_shortened_title_when_there_is_one() {
        let subject = (SubjectType::SUBJECT_TYPE_NOTE as i32, Uuid::now_v7());
        let long_title = "あ".repeat(100);
        let n = content_removed(Uuid::now_v7(), subject, &long_title, "違反");
        assert_eq!(n.subject, Some(subject));
        assert!(n.body.ja.contains(&format!("「{}…」", "あ".repeat(QUOTED_TITLE_CHARS - 1))));
        let n = content_removed(Uuid::now_v7(), subject, "  ", "違反");
        assert!(n.body.ja.starts_with("あなたの投稿は") && !n.body.ja.contains('「'));
    }

    #[test]
    fn report_resolved_is_inbox_only_and_says_whether_action_was_taken() {
        let case_id = Uuid::now_v7();
        let acted = report_resolved(Uuid::now_v7(), case_id, true);
        let dismissed = report_resolved(Uuid::now_v7(), case_id, false);
        assert!(acted.inbox && !acted.push);
        assert_eq!(acted.kind, KIND_REPORT_RESOLVED);
        assert_ne!(acted.body.ja, dismissed.body.ja);
        assert_eq!(acted.data["action_taken"], true);
        assert_eq!(dismissed.data["action_taken"], false);
    }

    #[test]
    fn announcement_links_to_the_news_page() {
        let id = Uuid::now_v7();
        let n = announcement(Uuid::now_v7(), id, Localized::new("新機能!", "New!"), Localized::same("s"), Some("https://x/i.webp"));
        assert_eq!(n.path, format!("/news/{id}"));
        assert_eq!(n.image_url.as_deref(), Some("https://x/i.webp"));
        assert_eq!(n.collapse_key.as_deref(), Some(format!("announcement:{id}").as_str()));
    }

    #[test]
    fn suspension_mail_is_localized_and_escapes_html() {
        let until = DateTime::parse_from_rfc3339("2026-12-01T00:00:00Z").unwrap().with_timezone(&Utc);
        let ja = suspension_mail("ja-JP", "Asia/Tokyo", "<b>スパム</b>", Some(until));
        assert!(ja.subject.contains("停止"));
        assert!(ja.text.contains("2026-12-01 09:00 (Asia/Tokyo)"), "{}", ja.text);
        assert!(ja.html.contains("&lt;b&gt;スパム&lt;/b&gt;") && !ja.html.contains("<b>"));

        let en = suspension_mail("en", "Nowhere/Invalid", "spam", None);
        assert!(en.subject.contains("suspended") && en.text.contains("permanently"));
        let en = suspension_mail("en", "Nowhere/Invalid", "spam", Some(until));
        assert!(en.text.contains("2026-12-01 00:00 (UTC)"));

        assert!(suspension_lifted_mail("ja").subject.contains("解除"));
        assert!(suspension_lifted_mail("fr").subject.contains("active again"));
    }
}
