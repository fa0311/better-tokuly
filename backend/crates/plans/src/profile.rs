//! What this domain adds to identity's profiles: the "ひま" card for viewers
//! in the status's audience, the couple counter both partners chose to show,
//! and the countdowns the owner pinned.
//!
//! Everything here is among friends: the cards are shown to the owner and to
//! their friends, never to strangers or anonymous visitors.
use async_trait::async_trait;
use tokuly_core::{outbound::Localized, prelude::*};
use tokuly_identity::ProfileExtension;
use tokuly_proto::{
    common::LocalizedText,
    identity::{Profile, ProfileCard},
};
use tokuly_social::graph;

use crate::{Plans, copy, countdowns::countdown_path, dates, people::name_of};

/// Position of this domain's cards among the cards of a profile.
const FREE_ORDER: i32 = 40;
const COUPLE_ORDER: i32 = 41;
const COUNTDOWN_ORDER: i32 = 42;

fn text(value: Localized) -> LocalizedText {
    LocalizedText { ja: value.ja, en: value.en, ..Default::default() }
}

/// The kind-specific values of a card, as whatever map the card uses.
fn data<M: FromIterator<(String, String)>, const N: usize>(pairs: [(&str, String); N]) -> M {
    pairs.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

#[async_trait]
impl ProfileExtension for Plans {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let core = &self.core;
        let Some(viewer) = viewer else { return Ok(()) };
        if viewer != owner && !graph::are_friends(core, owner, viewer).await? {
            return Ok(());
        }
        let now = now();
        let zone = dates::zone_of(core, owner).await?;

        if let Some(status) = self.visible_free_status(viewer, owner).await? {
            let (ja, en) =
                copy::free_line(copy::free_window(status.window(), status.starts_at, now, zone), &status.note);
            profile.cards.push(ProfileCard {
                kind: "plans.free".into(),
                title: text(Localized::new("ひま", "Free")).into(),
                text: text(Localized::new(ja, en)).into(),
                emoji: "🙌".into(),
                path: "/plans/free".into(),
                data: data([
                    ("status_id", status.id.to_string()),
                    ("starts_at", status.starts_at.to_rfc3339()),
                    ("ends_at", status.ends_at.to_rfc3339()),
                    ("note", status.note.clone()),
                    ("area_name", status.area_name.clone()),
                ]),
                order: FREE_ORDER,
                ..Default::default()
            });
        }

        if let Some(couple) = self.couple_on_profile(owner).await? {
            let partner_id = couple.partner_of(owner);
            // The partner is named on the card, so it is left out for
            // someone the partner is hidden from.
            if viewer == partner_id || !graph::is_blocked_either(core, viewer, partner_id).await? {
                let partner = tokuly_identity::summary(core, partner_id).await?;
                let days = couple.days(dates::day_in(zone, now));
                let title = if couple.title.is_empty() {
                    Localized::new("カップル", "Couple")
                } else {
                    Localized::same(couple.title.clone())
                };
                profile.cards.push(ProfileCard {
                    kind: "plans.anniversary".into(),
                    title: text(title).into(),
                    text: text(copy::couple_days(&name_of(&partner), days, couple.count_start_day)).into(),
                    emoji: if couple.emoji.is_empty() { "💑".into() } else { couple.emoji.clone() },
                    path: format!("/u/{}", partner.handle),
                    data: data([
                        ("anniversary_id", couple.id.to_string()),
                        ("partner_id", partner_id.to_string()),
                        ("start_date", dates::format_date(couple.start_date)),
                        ("days", days.to_string()),
                    ]),
                    order: COUPLE_ORDER,
                    ..Default::default()
                });
            }
        }

        for (position, (countdown, days_left)) in self.pinned_countdowns(owner).await?.into_iter().enumerate() {
            let (ja, en) = copy::days_left(days_left);
            profile.cards.push(ProfileCard {
                kind: "plans.countdown".into(),
                title: text(Localized::same(countdown.title.clone())).into(),
                text: text(Localized::new(ja, en)).into(),
                emoji: countdown.emoji.clone(),
                path: countdown_path(countdown.id),
                data: data([
                    ("countdown_id", countdown.id.to_string()),
                    ("date", dates::format_date(countdown.next_date(dates::day_in(zone, now)))),
                    ("days_left", days_left.to_string()),
                ]),
                order: COUNTDOWN_ORDER + position as i32,
                ..Default::default()
            });
        }
        Ok(())
    }
}
