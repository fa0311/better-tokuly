//! Hints: what the receiver of a flame can buy about whoever sent it.
//!
//! A flame has at most two hints, sold in order:
//!
//! 1. the first letter of the sender's name;
//! 2. the sender's school, or how many months the two have been friends.
//!
//! A hint is never the identity. That needs care in one case: a display name
//! of a single character (葵, 蓮) *is* its first letter, so for such names
//! the letter comes from the handle instead, which always has at least three.
//!
//! The sender decides whether hints are sold at all (`allow_hints`). Nothing
//! is charged for a hint that cannot be given, and a hint, once bought, is
//! stored and never changes. Rows of `ask_flame_hints` without an `entry_id`
//! are purchases in flight (see [`Ask::purchase_hint`]): nobody is shown them.
//!
//! This is one of the two places that read the sender of a flame (the other
//! is the moderation service). The sender's ID stays inside
//! [`Ask::hint_material`], which returns only the hint itself.
use std::time::Duration;

use chrono::Utc;
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

/// A hint as stored.
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

fn hints_unavailable() -> AppError {
    AppError::precondition("HINTS_UNAVAILABLE", "no hints are available for this flame")
}

impl Ask {
    /// The hint at `position` for a flame: its kind and value. `None` when
    /// the sender's account is gone or the sender does not allow hints.
    ///
    /// The sender is read and used here, and only what the hint says leaves
    /// this function.
    async fn hint_material(&self, flame_id: Uuid, position: usize) -> AppResult<Option<(i16, String)>> {
        #[derive(sqlx::FromRow)]
        struct Sender {
            display_name: String,
            handle: String,
            school_name: String,
            allow_hints: bool,
            friends_since: Option<chrono::DateTime<Utc>>,
        }
        let sender: Option<Sender> = sqlx::query_as(
            "SELECT u.display_name, u.handle::text AS handle, u.school_name,
                    COALESCE((SELECT s.allow_hints FROM ask_settings s WHERE s.user_id = u.id), true) AS allow_hints,
                    COALESCE(
                        (SELECT l.since FROM ask_friend_links l WHERE l.user_id = f.receiver_id AND l.friend_id = u.id),
                        (SELECT e.created_at FROM ask_poll_exposure e WHERE e.user_id = u.id AND e.friend_id = f.receiver_id),
                        f.created_at
                    ) AS friends_since
             FROM ask_flames f JOIN users u ON u.id = f.sender_id
             WHERE f.id = $1 AND u.deleted_at IS NULL AND u.banned_at IS NULL",
        )
        .bind(flame_id)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(sender) = sender.filter(|sender| sender.allow_hints) else { return Ok(None) };
        Ok(match position {
            0 => Some((INITIAL, initial_of(&sender.display_name, &sender.handle)))
                .filter(|(_, letter)| !letter.is_empty()),
            1 if !sender.school_name.trim().is_empty() => Some((SCHOOL, sender.school_name.trim().to_owned())),
            1 => Some((
                FRIEND_MONTHS,
                months_between(sender.friends_since.unwrap_or_else(Utc::now), Utc::now()).to_string(),
            )),
            _ => None,
        })
    }

    /// A hint that was paid for: the one bought under `by_key`, or the one
    /// at `by_position`.
    async fn paid_hint(
        &self,
        flame_id: Uuid,
        by_key: Option<&str>,
        by_position: Option<i16>,
    ) -> AppResult<Option<HintRow>> {
        Ok(sqlx::query_as(
            "SELECT flame_id, kind, value FROM ask_flame_hints
             WHERE flame_id = $1 AND entry_id IS NOT NULL
               AND ($2::text IS NULL OR idempotency_key = $2) AND ($3::smallint IS NULL OR position = $3)",
        )
        .bind(flame_id)
        .bind(by_key)
        .bind(by_position)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// Makes sure an unpaid hint is stored at `position` before anything is
    /// charged: the one an interrupted purchase left behind, or a new one.
    /// Fails with `HINTS_UNAVAILABLE` when there is none and the sender does
    /// not (or can no longer) give one.
    async fn reserve_hint(&self, flame_id: Uuid, position: usize, price: i64, key: &str) -> AppResult<()> {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM ask_flame_hints WHERE flame_id = $1 AND position = $2 AND entry_id IS NULL)",
        )
        .bind(flame_id)
        .bind(position as i16)
        .fetch_one(&self.core.db)
        .await?;
        if waiting {
            return Ok(());
        }
        let (kind, value) = self.hint_material(flame_id, position).await?.ok_or_else(hints_unavailable)?;
        sqlx::query(
            "INSERT INTO ask_flame_hints (flame_id, position, kind, value, price, idempotency_key)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT DO NOTHING",
        )
        .bind(flame_id)
        .bind(position as i16)
        .bind(kind)
        .bind(&value)
        .bind(price)
        .bind(key)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Buys the next hint about the sender of one of `receiver`'s flames.
    /// Returns the flame with its hints, the hint this purchase is for, and
    /// the coins this call charged (0 when it repeated an earlier one).
    ///
    /// The hint is stored, unpaid and invisible, before the wallet is
    /// charged, and marked paid afterwards. If the process stops in between,
    /// the next purchase finds the stored hint and pays for it under the
    /// same ledger key, which charges only if the first attempt had not: the
    /// buyer never pays without getting the hint, and never pays twice.
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

        let (hint, charged) = match self.paid_hint(flame_id, Some(key), None).await? {
            // A retry of a purchase that already went through.
            Some(hint) => (hint, 0),
            None => {
                let bought: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM ask_flame_hints WHERE flame_id = $1 AND entry_id IS NOT NULL",
                )
                .bind(flame_id)
                .fetch_one(&self.core.db)
                .await?;
                let position = bought as usize;
                // The price is looked at before the sender is: whether a
                // further hint exists must not depend on who sent the flame.
                let price = self.next_hint_price(position);
                ensure!(price > 0, precondition, "NO_MORE_HINTS", "there are no more hints for this flame");
                self.reserve_hint(flame_id, position, price, key).await?;
                // One charge per flame and hint, however the request is
                // repeated. The memo is shown in the buyer's coin history
                // and says nothing about the sender.
                let memo = serde_json::json!({ "flame_id": flame_id, "hint": position + 1 });
                let spent = ledger::spend(
                    &self.core,
                    receiver,
                    price,
                    HINT_REASON,
                    &format!("{HINT_REASON}:{flame_id}:{position}"),
                    memo,
                )
                .await;
                let (entry_id, applied) = match spent {
                    Ok(spent) => spent,
                    Err(e) => {
                        // Nothing was charged: take back the hint this
                        // request stored (not one an earlier, interrupted
                        // purchase may have paid for).
                        sqlx::query("DELETE FROM ask_flame_hints WHERE flame_id = $1 AND position = $2 AND entry_id IS NULL AND idempotency_key = $3")
                            .bind(flame_id)
                            .bind(position as i16)
                            .bind(key)
                            .execute(&self.core.db)
                            .await?;
                        return Err(e);
                    }
                };
                sqlx::query("UPDATE ask_flame_hints SET entry_id = $3, idempotency_key = $4 WHERE flame_id = $1 AND position = $2 AND entry_id IS NULL")
                    .bind(flame_id)
                    .bind(position as i16)
                    .bind(entry_id)
                    .bind(key)
                    .execute(&self.core.db)
                    .await?;
                // Whichever request paid for this position's hint, that is the hint.
                let hint = self.paid_hint(flame_id, None, Some(position as i16)).await?.ok_or_else(|| {
                    AppError::internal(anyhow::anyhow!(
                        "hint {position} of flame {flame_id} was paid for but not stored"
                    ))
                })?;
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
