//! The question box on profiles: a card on the profile of everyone whose
//! box is open, leading to the box and its Q&A page.
use async_trait::async_trait;
use tokuly_core::prelude::*;
use tokuly_identity::ProfileExtension;
use tokuly_proto::{
    common::LocalizedText,
    identity::{Profile, ProfileCard},
};
use tokuly_social::graph;

use crate::{Ask, settings};

/// Kind of the card.
pub(crate) const CARD_KIND: &str = "ask.box";
/// Position among a profile's cards.
const CARD_ORDER: i32 = 40;

#[async_trait]
impl ProfileExtension for Ask {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let settings = settings::load(&self.core.db, owner).await?;
        if !settings.box_open {
            return Ok(());
        }
        // Identity does not build a profile for someone in a block with its
        // owner; should it ever, the box stays out of it.
        let is_owner = viewer == Some(owner);
        if let Some(viewer) = viewer
            && !is_owner
            && graph::is_blocked_either(&self.core, viewer, owner).await?
        {
            return Ok(());
        }
        let Some(handle) = profile.user.as_option().map(|user| user.handle.clone()).filter(|handle| !handle.is_empty())
        else {
            return Ok(());
        };
        let can_ask = match viewer {
            Some(viewer) if !is_owner => {
                !settings.friends_only() || graph::are_friends(&self.core, owner, viewer).await?
            }
            _ => false,
        };
        let answers = self.answer_count(owner, self.page_audiences(owner, viewer).await?).await?;
        // The owner's own line is the same in both languages; the default has two.
        let text = if settings.box_prompt.is_empty() {
            LocalizedText {
                ja: settings::DEFAULT_BOX_PROMPT.0.to_owned(),
                en: settings::DEFAULT_BOX_PROMPT.1.to_owned(),
                ..Default::default()
            }
        } else {
            LocalizedText { ja: settings.box_prompt.clone(), en: settings.box_prompt.clone(), ..Default::default() }
        };
        profile.cards.push(ProfileCard {
            kind: CARD_KIND.to_owned(),
            title: LocalizedText { ja: "質問箱".to_owned(), en: "Question box".to_owned(), ..Default::default() }
                .into(),
            text: text.into(),
            emoji: "💌".to_owned(),
            path: format!("/u/{handle}/ask"),
            data: [
                ("audience".to_owned(), if settings.friends_only() { "friends" } else { "signed_in" }.to_owned()),
                ("can_ask".to_owned(), can_ask.to_string()),
                ("answer_count".to_owned(), answers.to_string()),
            ]
            .into_iter()
            .collect(),
            order: CARD_ORDER,
            ..Default::default()
        });
        Ok(())
    }
}
