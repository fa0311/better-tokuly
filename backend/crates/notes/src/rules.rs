//! The posting rules (the page people agree to before their first note) and
//! who agreed to which version of them.
//!
//! The text lives here. Changing it in a way people should read again means
//! raising [`RULES_VERSION`]: everyone is then asked to agree once more
//! before their next note.
use chrono::{DateTime, Utc};
use tokuly_core::{outbound::Localized, prelude::*};
use tokuly_proto::notes::NoteRule;

/// Version of [`RULES`]. Posting needs an agreement to exactly this version.
pub const RULES_VERSION: i32 = 1;

/// `(title, body)`, each as `(Japanese, English)`.
const RULES: &[((&str, &str), (&str, &str))] = &[
    (
        ("思いやりを持って投稿しよう!", "Be kind"),
        (
            "悪口・いやがらせ・差別的な投稿はやめよう。画面の向こうにいるのは、あなたと同じ人だよ。",
            "No insults, harassment or hate. There is a person just like you on the other side of the screen.",
        ),
    ),
    (
        ("個人情報は書かないでね", "Keep personal information private"),
        (
            "自分や他の人の住所・電話番号・学校名・くわしい居場所を載せないようにしよう。",
            "Do not post addresses, phone numbers, school names or precise whereabouts — yours or anyone else's.",
        ),
    ),
    (
        ("写真や動画は許可をもらってから", "Ask before you post someone"),
        (
            "友達が写っている写真や動画は、本人に聞いてから投稿しよう。刺激の強い内容には注意書き(CW)をつけてね。",
            "Ask the people in your photos and videos before posting them, and put a content warning on sensitive material.",
        ),
    ),
    (
        ("危険なこと・違法なことは投稿しない", "Nothing dangerous or illegal"),
        (
            "暴力的・性的な内容、危険な行為、法律に違反する内容は投稿できません。",
            "Violent or sexual content, dangerous acts and anything against the law do not belong here.",
        ),
    ),
    (
        ("なりすまし・スパムは禁止", "No impersonation, no spam"),
        (
            "他の人のふりをしたり、同じ内容を何度も投稿したり、宣伝ばかりするのはやめよう。",
            "Do not pretend to be someone else, post the same thing over and over, or advertise.",
        ),
    ),
    (
        ("困ったときは通報してね", "Report what does not belong"),
        (
            "ルールに合わない投稿を見つけたら通報しよう。違反した投稿は削除され、アカウントが停止されることがあります。",
            "Report notes that break these rules. They are removed, and the accounts behind them may be suspended.",
        ),
    ),
];

/// The rules in the caller's language (`Accept-Language`; Japanese unless
/// another language is asked for).
pub fn rules(language: &str) -> Vec<NoteRule> {
    let language = if language.is_empty() { "ja" } else { language };
    RULES
        .iter()
        .map(|(title, body)| NoteRule {
            title: Localized::new(title.0, title.1).pick(language).to_owned(),
            body: Localized::new(body.0, body.1).pick(language).to_owned(),
            ..Default::default()
        })
        .collect()
}

/// The version a user agreed to last and when; `None` if they never did.
pub async fn agreement(core: &Core, user_id: Uuid) -> AppResult<Option<(i32, DateTime<Utc>)>> {
    Ok(sqlx::query_as("SELECT version, agreed_at FROM note_rule_agreements WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&core.db)
        .await?)
}

/// Records that a user agrees to the current rules. Agreeing again keeps
/// the first date.
pub async fn agree(core: &Core, user_id: Uuid) -> AppResult<DateTime<Utc>> {
    Ok(sqlx::query_scalar(
        "INSERT INTO note_rule_agreements (user_id, version) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET
             agreed_at = CASE WHEN note_rule_agreements.version = EXCLUDED.version THEN note_rule_agreements.agreed_at ELSE now() END,
             version = EXCLUDED.version
         RETURNING agreed_at",
    )
    .bind(user_id)
    .bind(RULES_VERSION)
    .fetch_one(&core.db)
    .await?)
}

/// Fails with `RULES_NOT_ACCEPTED` unless the user agreed to the current rules.
pub async fn ensure_agreed(core: &Core, user_id: Uuid) -> AppResult<()> {
    let agreed = agreement(core, user_id).await?.is_some_and(|(version, _)| version == RULES_VERSION);
    ensure!(agreed, precondition, "RULES_NOT_ACCEPTED", "agree to the note rules before posting");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_come_in_both_languages() {
        let (ja, en) = (rules("ja"), rules("en-US"));
        assert_eq!(ja.len(), RULES.len());
        assert_eq!(en.len(), RULES.len());
        assert!(ja.iter().chain(&en).all(|rule| !rule.title.is_empty() && !rule.body.is_empty()));
        assert_eq!(ja[0].title, "思いやりを持って投稿しよう!");
        assert_eq!(en[0].title, "Be kind");
        // Japanese first: no stated preference reads Japanese.
        assert_eq!(rules("")[0].title, ja[0].title);
    }
}
