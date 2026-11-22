//! What the emoji domain plugs into the generic domains: the subject
//! resolver (so emoji can be reported), the personal data export, and a
//! card on the profile of people whose emoji are in the catalogue.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_identity::{Identity, ProfileExtension};
use tokuly_proto::{
    common::{LocalizedText, SubjectType},
    identity::{Profile, ProfileCard},
};

use crate::{
    emojis,
    model::{Viewer, shortcode},
};

/// Position of the emoji card among a profile's cards.
const PROFILE_CARD_ORDER: i32 = 60;
/// Names listed on the profile card.
const PROFILE_CARD_NAMES: i64 = 5;

/// Registers the resolver, the exporter and the profile extension. Called
/// once from `Emoji::new`.
pub fn register(core: &Core, identity: &Identity) {
    core.subjects.register(SubjectType::SUBJECT_TYPE_CUSTOM_EMOJI, Arc::new(EmojiSubjects { core: core.clone() }));
    core.exporters.register(Arc::new(EmojiExporter { core: core.clone() }));
    identity.add_profile_extension(Arc::new(AuthorCard { core: core.clone() }));
}

/// `SubjectInfo::owner_id` of emoji nobody is credited for (imports,
/// uncredited additions, deleted accounts): there is no user to notify.
const NO_OWNER: Uuid = Uuid::nil();

struct EmojiSubjects {
    core: Core,
}

#[async_trait]
impl SubjectResolver for EmojiSubjects {
    /// `id` is an emoji ID or a name. Approved emoji resolve for everyone;
    /// an author also resolves their own emoji in other states.
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Some(emoji) = emojis::find(&self.core, id).await? else { return Ok(None) };
        // Generic domains know viewers only by ID, so nobody is a moderator here.
        let viewer = viewer.map(Viewer::user).unwrap_or_default();
        if !viewer.can_see(&emoji) {
            return Ok(None);
        }
        Ok(Some(SubjectInfo {
            id: emoji.id,
            owner_id: emoji.author_id.unwrap_or(NO_OWNER),
            title: emoji.shortcode(),
            path: emoji.path(),
            image_url: Some(emoji.image().url).filter(|url| !url.is_empty()),
            // An emoji can be reported, not liked or commented on.
            interactions_enabled: false,
        }))
    }
}

struct EmojiExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for EmojiExporter {
    fn name(&self) -> &'static str {
        "emoji"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        let deck: Option<Vec<String>> = sqlx::query_scalar("SELECT items FROM emoji_decks WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
        Ok(serde_json::json!({
            "emojis": section(
                "SELECT jsonb_build_object('id', id, 'name', name, 'state', state, 'category', category, 'aliases', aliases,
                        'license', license, 'sensitive', sensitive, 'image_url', image ->> 'url', 'review_note', review_note,
                        'created_at', created_at, 'approved_at', approved_at)
                 FROM emojis WHERE author_id = $1 ORDER BY id"
            ).await?,
            "deck": deck,
            "recently_used": section(
                "SELECT jsonb_build_object('emoji', item, 'used_at', used_at, 'use_count', use_count)
                 FROM emoji_recents WHERE user_id = $1 ORDER BY used_at DESC"
            ).await?,
            "trusted_submitter": emojis::is_trusted(db, user_id).await?,
        }))
    }
}

/// Shows "made N emoji" on the profile of people with emoji in the
/// catalogue. Who made an emoji is public (it is on the emoji's page), so
/// the card is shown to everyone who may see the profile.
struct AuthorCard {
    core: Core,
}

#[async_trait]
impl ProfileExtension for AuthorCard {
    async fn enrich_profile(&self, _viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let newest: Vec<(i64, String, serde_json::Value)> = sqlx::query_as(
            "SELECT count(*) OVER (), name, image FROM emojis
             WHERE author_id = $1 AND state = 2
             ORDER BY approved_at DESC, id DESC
             LIMIT $2",
        )
        .bind(owner)
        .bind(PROFILE_CARD_NAMES)
        .fetch_all(&self.core.db)
        .await?;
        let Some((count, _, image)) = newest.first() else { return Ok(()) };
        let names: Vec<String> = newest.iter().map(|(_, name, _)| shortcode(name)).collect();
        profile.cards.push(ProfileCard {
            kind: "emoji.author".to_owned(),
            title: Some(LocalizedText {
                ja: "作った絵文字".to_owned(),
                en: "Emoji made".to_owned(),
                ..Default::default()
            })
            .into(),
            text: Some(LocalizedText { ja: format!("{count}個"), en: format!("{count} emoji"), ..Default::default() })
                .into(),
            image: serde_json::from_value::<tokuly_proto::common::Image>(image.clone()).ok().into(),
            path: format!("/emoji?author={owner}"),
            data: [("count".to_owned(), count.to_string()), ("newest".to_owned(), names.join(" "))]
                .into_iter()
                .collect(),
            order: PROFILE_CARD_ORDER,
            ..Default::default()
        });
        Ok(())
    }
}
