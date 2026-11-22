//! Hints: what the receiver of a flame can buy about whoever sent it.
//!
//! A flame sells at most two hints, in order:
//!
//! 1. the first letter of the sender's name;
//! 2. the sender's school, or how many months the two have been friends.
//!
//! A hint is never the identity. That needs care in one case: a display name
//! of a single character (葵, 蓮) *is* its first letter, so for such names
//! the letter comes from the handle instead, which always has at least three.
//!
//! # Decided when the vote is cast
//!
//! Whether a flame sells hints (the voter's `allow_hints`) and what they say
//! is worked out once, by [`Ask::hints_on_offer`] while the vote is cast, and
//! stored on the flame. A purchase reads the flame and nothing else: it
//! never looks at the sender's account.
//!
//! If it did, buying would be a way to watch that account. The price list
//! and `HINTS_UNAVAILABLE` would change when the sender switched hints off,
//! was suspended or deleted their account, and the receiver, seeing which of
//! their flames changed on the day a friend disappeared, would know who sent
//! them, without paying and without a trace.
//!
//! A hint that was bought is copied to `ask_flame_hints` with the wallet
//! entry that paid for it; nothing is charged for a hint that is not given.
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_proto::{
    ask::{Flame, Hint, HintKind},
    common::LocalizedText,
};
use tokuly_wallet::ledger;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{Ask, friends::months_between};

/// Ledger reason of a hint purchase.
pub(crate) const HINT_REASON: &str = "ask.hint";
/// Hint purchases one user may attempt per minute.
const PURCHASES_PER_MINUTE: u32 = 30;

const INITIAL: i16 = HintKind::HINT_KIND_INITIAL as i16;
const SCHOOL: i16 = HintKind::HINT_KIND_SCHOOL as i16;
const FRIEND_MONTHS: i16 = HintKind::HINT_KIND_FRIEND_MONTHS as i16;

/// A hint that was bought.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct HintRow {
    pub flame_id: Uuid,
    pub kind: i16,
    pub value: String,
}

impl HintRow {
    pub(crate) fn to_proto(&self) -> Hint {
        Hint {
            kind: i32::from(self.kind).into(),
            text: sentence(self.kind, &self.value).into(),
            value: self.value.clone(),
            ..Default::default()
        }
    }
}

/// The hint as a sentence in both languages.
fn sentence(kind: i16, value: &str) -> LocalizedText {
    let (ja, en) = match kind {
        INITIAL => {
            (format!("名前の最初の文字は「{value}」だよ"), format!("Their name starts with \u{201c}{value}\u{201d}."))
        }
        SCHOOL => (format!("{value}の人だよ"), format!("They go to {value}.")),
        FRIEND_MONTHS if value == "0" => {
            ("友達になってまだ1か月たってないよ".to_owned(), "You have been friends for less than a month.".to_owned())
        }
        FRIEND_MONTHS => {
            (format!("友達になって{value}か月以上だよ"), format!("You have been friends for {value} months or more."))
        }
        _ => (value.to_owned(), value.to_owned()),
    };
    LocalizedText { ja, en, ..Default::default() }
}

/// The first letter of a name, without ever being the whole name: taken
/// from the display name when it has two letters or more, otherwise from
/// the handle (three characters at least). Latin letters are upper-cased.
pub(crate) fn initial_of(display_name: &str, handle: &str) -> String {
    let name = display_name.trim();
    let source = if name.graphemes(true).count() >= 2 { name } else { handle.trim().trim_start_matches('@') };
    source.graphemes(true).next().map(|first| first.to_uppercase()).unwrap_or_default()
}

/// What a voter's flame sells, in order, as `(kind, value)`.
fn hints_about(
    display_name: &str,
    handle: &str,
    school_name: &str,
    friends_since: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<(i16, String)> {
    let letter = initial_of(display_name, handle);
    if letter.is_empty() {
        return Vec::new();
    }
    let school = school_name.trim();
    let second = if school.is_empty() {
        (FRIEND_MONTHS, months_between(friends_since, now).to_string())
    } else {
        (SCHOOL, school.to_owned())
    };
    vec![(INITIAL, letter), second]
}

/// Hints as stored on a flame (`ask_flames.hints`).
fn stored(hints: &[(i16, String)]) -> serde_json::Value {
    hints.iter().map(|(kind, value)| serde_json::json!({ "kind": kind, "value": value })).collect()
}

/// The buyer's key: 8 to 128 characters.
fn check_key(key: &str) -> AppResult<()> {
    ensure!(
        (8..=128).contains(&key.chars().count()),
        invalid,
        "IDEMPOTENCY_KEY_INVALID",
        "an idempotency key of 8-128 characters is required"
    );
    Ok(())
}

impl Ask {
    /// The price of the hint after `bought` hints of a flame that sells
    /// `on_offer`; 0 when none is left.
    pub(crate) fn next_hint_price(&self, bought: usize, on_offer: usize) -> i64 {
        if bought < on_offer { self.hint_prices.get(bought).copied().unwrap_or(0) } else { 0 }
    }

    /// What a vote of `voter` for `receiver` sells as hints, in the form
    /// stored on the flame: nothing when the voter switched hints off.
    ///
    /// This is the one place that reads a voter's account for hints, and it
    /// runs once per vote, before the flame exists.
    pub(crate) async fn hints_on_offer(&self, voter: Uuid, receiver: Uuid) -> AppResult<serde_json::Value> {
        #[derive(sqlx::FromRow)]
        struct Voter {
            display_name: String,
            handle: String,
            school_name: String,
            allow_hints: bool,
            friends_since: Option<DateTime<Utc>>,
        }
        let voter: Option<Voter> = sqlx::query_as(
            "SELECT u.display_name, u.handle::text AS handle, u.school_name,
                    COALESCE((SELECT s.allow_hints FROM ask_settings s WHERE s.user_id = u.id), true) AS allow_hints,
                    COALESCE(
                        (SELECT l.since FROM ask_friend_links l WHERE l.user_id = $2 AND l.friend_id = u.id),
                        (SELECT e.created_at FROM ask_poll_exposure e WHERE e.user_id = u.id AND e.friend_id = $2)
                    ) AS friends_since
             FROM users u WHERE u.id = $1",
        )
        .bind(voter)
        .bind(receiver)
        .fetch_optional(&self.core.db)
        .await?;
        let now = Utc::now();
        let hints = match voter.filter(|voter| voter.allow_hints) {
            Some(voter) => hints_about(
                &voter.display_name,
                &voter.handle,
                &voter.school_name,
                voter.friends_since.unwrap_or(now),
                now,
            ),
            None => Vec::new(),
        };
        Ok(stored(&hints[..hints.len().min(self.hint_prices.len())]))
    }

    /// The hint of a flame that was bought under `key`.
    async fn hint_bought_with(&self, flame_id: Uuid, key: &str) -> AppResult<Option<HintRow>> {
        Ok(sqlx::query_as(
            "SELECT flame_id, kind, value FROM ask_flame_hints WHERE flame_id = $1 AND $2 = ANY(request_keys)",
        )
        .bind(flame_id)
        .bind(key)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// Buys the next hint about the sender of one of `receiver`'s flames.
    /// Returns the flame with its hints, the hint this purchase is for, and
    /// the coins this call charged (0 when it repeated an earlier one).
    ///
    /// Everything comes from the flame. The wallet is charged under a key
    /// made of the flame and the hint's position, so that however the
    /// request is repeated, and however many arrive at once, one hint is
    /// paid for once; the hint is stored after the charge, and a request
    /// that was interrupted in between finds the charge made and stores it
    /// without paying again.
    pub(crate) async fn purchase_hint(
        &self,
        receiver: Uuid,
        flame_id: Uuid,
        key: &str,
    ) -> AppResult<(Flame, Hint, i64)> {
        check_key(key)?;
        self.core
            .cache
            .rate_limit(&format!("ask:hint:{receiver}"), PURCHASES_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let flame = self.received_flame(receiver, flame_id).await?;

        let (hint, charged) = match self.hint_bought_with(flame_id, key).await? {
            // A retry of a purchase that already went through.
            Some(hint) => (hint, 0),
            None => {
                let on_offer = flame.hints_on_offer as usize;
                ensure!(on_offer > 0, precondition, "HINTS_UNAVAILABLE", "this flame sells no hints");
                let bought: i64 = sqlx::query_scalar("SELECT count(*) FROM ask_flame_hints WHERE flame_id = $1")
                    .bind(flame_id)
                    .fetch_one(&self.core.db)
                    .await?;
                let position = bought as usize;
                let price = self.next_hint_price(position, on_offer);
                ensure!(price > 0, precondition, "NO_MORE_HINTS", "there are no more hints for this flame");
                let (kind, value): (i16, String) = sqlx::query_as(
                    "SELECT (hints -> $2::int ->> 'kind')::smallint, hints -> $2::int ->> 'value' FROM ask_flames WHERE id = $1",
                )
                .bind(flame_id)
                .bind(position as i32)
                .fetch_one(&self.core.db)
                .await?;
                // The memo is shown in the buyer's coin history and says
                // nothing about the sender.
                let memo = serde_json::json!({ "flame_id": flame_id, "hint": position + 1 });
                let (entry_id, applied) = ledger::spend(
                    &self.core,
                    receiver,
                    price,
                    HINT_REASON,
                    &format!("{HINT_REASON}:{flame_id}:{position}"),
                    memo,
                )
                .await?;
                // Whichever request stored this position's hint first, every
                // request that asked for it is remembered as having bought it.
                let hint: HintRow = sqlx::query_as(
                    "INSERT INTO ask_flame_hints AS h (flame_id, position, kind, value, price, request_keys, entry_id)
                     VALUES ($1, $2, $3, $4, $5, ARRAY[$6::text], $7)
                     ON CONFLICT (flame_id, position) DO UPDATE SET
                         request_keys = CASE WHEN $6::text = ANY(h.request_keys) THEN h.request_keys ELSE array_append(h.request_keys, $6::text) END
                     RETURNING flame_id, kind, value",
                )
                .bind(flame_id)
                .bind(position as i16)
                .bind(kind)
                .bind(&value)
                .bind(price)
                .bind(key)
                .bind(entry_id)
                .fetch_one(&self.core.db)
                .await?;
                (hint, if applied { price } else { 0 })
            }
        };
        let mut flames = self.render_flames(std::slice::from_ref(&flame)).await?;
        let flame = flames.pop().ok_or_else(|| AppError::internal(anyhow::anyhow!("a rendered flame went missing")))?;
        Ok((flame, hint.to_proto(), charged))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration as Days;

    use super::*;

    #[test]
    fn the_initial_is_the_first_letter_upper_cased() {
        assert_eq!(initial_of("hanako", "hana_2009"), "H");
        assert_eq!(initial_of("たろう", "taro"), "た");
        assert_eq!(initial_of("  Émile ", "emile"), "É");
        assert_eq!(initial_of("山田 花子", "yamada"), "山");
    }

    #[test]
    fn a_one_letter_name_takes_its_letter_from_the_handle() {
        // 葵's first letter would be her whole name: the handle's is used.
        assert_eq!(initial_of("葵", "aoi_0412"), "A");
        assert_eq!(initial_of(" 蓮 ", "ren"), "R");
        assert_eq!(initial_of("🌸", "sakura"), "S");
        assert_eq!(initial_of("", "mio"), "M");
        for (name, handle) in [("葵", "aoi_0412"), ("蓮", "ren"), ("🌸", "sakura")] {
            assert_ne!(initial_of(name, handle), name, "{name}: the hint must not be the full name");
        }
        // A one-letter Latin name can share its letter with the handle. The
        // hint is still the handle's initial and says no more than "starts
        // with K", which fits every Kenta and Kana as well.
        assert_eq!(initial_of("K", "kei"), "K");
        assert_eq!(initial_of("K", "taro"), "T");
    }

    #[test]
    fn an_emoji_sequence_counts_as_one_letter_and_is_not_cut_in_half() {
        // One grapheme made of several code points: a single-letter name.
        assert_eq!(initial_of("👩‍🦰", "red"), "R");
        // Two graphemes: the first one, whole.
        assert_eq!(initial_of("👩‍🦰みか", "mika"), "👩‍🦰");
    }

    #[test]
    fn a_vote_sells_the_initial_and_then_the_school_or_the_months() {
        let now = Utc::now();
        let with_school = hints_about("hanako", "hana", " 桜丘高校 ", now - Days::days(400), now);
        assert_eq!(with_school, vec![(INITIAL, "H".to_owned()), (SCHOOL, "桜丘高校".to_owned())]);
        let without = hints_about("hanako", "hana", "  ", now - Days::days(95), now);
        assert_eq!(without, vec![(INITIAL, "H".to_owned()), (FRIEND_MONTHS, "3".to_owned())]);
        // No letter to give (which a real account cannot have): no hints.
        assert!(hints_about("", "", "桜丘高校", now, now).is_empty());
    }

    #[test]
    fn hints_are_stored_on_the_flame_in_order() {
        let hints = vec![(INITIAL, "H".to_owned()), (SCHOOL, "桜丘高校".to_owned())];
        assert_eq!(
            stored(&hints),
            serde_json::json!([{ "kind": 1, "value": "H" }, { "kind": 2, "value": "桜丘高校" }])
        );
        assert_eq!(stored(&[]), serde_json::json!([]));
    }

    #[test]
    fn hints_read_as_sentences_in_both_languages() {
        let initial = HintRow { flame_id: new_id(), kind: INITIAL, value: "H".into() }.to_proto();
        assert_eq!(initial.kind, HintKind::HINT_KIND_INITIAL);
        assert_eq!(initial.value, "H");
        let text = initial.text.as_option().unwrap();
        assert!(text.ja.contains("「H」") && text.en.contains('H'));

        let school = sentence(SCHOOL, "桜丘高校");
        assert!(school.ja.starts_with("桜丘高校") && school.en.contains("桜丘高校"));
        assert!(sentence(FRIEND_MONTHS, "0").en.contains("less than a month"));
        assert!(sentence(FRIEND_MONTHS, "7").ja.contains("7か月"));
    }

    #[test]
    fn keys_have_eight_to_128_characters() {
        assert!(check_key("12345678").is_ok());
        assert!(check_key(&"k".repeat(128)).is_ok());
        for bad in ["", "short", &"k".repeat(129)] {
            assert_eq!(check_key(bad).unwrap_err().reason, "IDEMPOTENCY_KEY_INVALID");
        }
    }
}
