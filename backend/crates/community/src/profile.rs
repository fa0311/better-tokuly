//! The profile card `community.memberships`: the public communities a user
//! chose to show on their profile.
//!
//! Showing a membership is opt-in per community (`UpdateMembership`), and
//! only memberships of public communities ever show: a class or a club says
//! where someone goes to school.
use async_trait::async_trait;
use tokuly_core::prelude::*;
use tokuly_identity::ProfileExtension;
use tokuly_proto::{
    common::LocalizedText,
    identity::{Profile, ProfileCard},
};
use tokuly_social::graph;

use crate::Community;

/// Communities named on the card (the count covers all of them).
const NAMED: i64 = 3;
/// Position among the cards of other domains.
const CARD_ORDER: i32 = 60;

/// The card's text: up to three names, and how many more there are.
fn card_text(names: &[String], total: i64) -> LocalizedText {
    let more = total - names.len() as i64;
    let (ja, en) = if more > 0 {
        (format!("{} ほか{more}件", names.join("、")), format!("{} and {more} more", names.join(", ")))
    } else {
        (names.join("、"), names.join(", "))
    };
    LocalizedText { ja, en, ..Default::default() }
}

#[async_trait]
impl ProfileExtension for Community {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        // Identity already refuses blocked viewers; this card never widens that.
        if let Some(viewer) = viewer.filter(|viewer| *viewer != owner)
            && graph::is_blocked_either(&self.core, viewer, owner).await?
        {
            return Ok(());
        }
        let shown: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT c.name, c.handle::text, count(*) OVER ()
             FROM community_members m JOIN communities c ON c.id = m.community_id
             WHERE m.user_id = $1 AND m.show_on_profile AND c.visibility = 1 AND c.deleted_at IS NULL
             ORDER BY c.member_count DESC, c.id DESC LIMIT $2",
        )
        .bind(owner)
        .bind(NAMED)
        .fetch_all(&self.core.db)
        .await?;
        let Some(total) = shown.first().map(|(_, _, total)| *total) else { return Ok(()) };
        let names: Vec<String> = shown.iter().map(|(name, ..)| name.clone()).collect();
        let handles: Vec<&str> = shown.iter().map(|(_, handle, _)| handle.as_str()).collect();
        profile.cards.push(ProfileCard {
            kind: "community.memberships".into(),
            title: LocalizedText { ja: "コミュニティ".into(), en: "Communities".into(), ..Default::default() }.into(),
            text: card_text(&names, total).into(),
            emoji: "🏘️".into(),
            path: format!("/communities?member={owner}"),
            data: [("count".to_owned(), total.to_string()), ("handles".to_owned(), handles.join(","))]
                .into_iter()
                .collect(),
            order: CARD_ORDER,
            ..Default::default()
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_names_a_few_and_counts_the_rest() {
        let names = vec!["Apex部".to_owned(), "3年B組".to_owned()];
        let text = card_text(&names, 2);
        assert_eq!(text.ja, "Apex部、3年B組");
        assert_eq!(text.en, "Apex部, 3年B組");
        let text = card_text(&names, 5);
        assert_eq!(text.ja, "Apex部、3年B組 ほか3件");
        assert_eq!(text.en, "Apex部, 3年B組 and 3 more");
    }
}
