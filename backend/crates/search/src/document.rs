//! The shape of an indexed document and its ranking signal.
use serde::{Deserialize, Serialize};
use tokuly_core::outbound::SearchDoc;
use uuid::Uuid;

use crate::{kinds::Kind, text::clean};

const MAX_TITLE_CHARS: usize = 200;
const MAX_TEXT_CHARS: usize = 8000;
const MAX_HANDLE_CHARS: usize = 64;
const MAX_TAGS: usize = 50;
const MAX_TAG_CHARS: usize = 64;
const MAX_PATH_CHARS: usize = 512;
const MAX_URL_CHARS: usize = 2048;

/// 2024-01-01T00:00:00Z. Ranks count time from here so the numbers stay small.
const RANK_EPOCH: i64 = 1_704_067_200;

/// A document as the registry stores it and Meilisearch indexes it.
///
/// Built from the publishing domain's [`SearchDoc`]: texts are cleaned (see
/// [`crate::text::clean`]) and bounded, and `extra.handle`, when present, is
/// lifted into the searchable `handle` attribute (typos are not tolerated on
/// handles). The indexer adds `rank` when it writes the document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexDoc {
    pub id: Uuid,
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub owner_id: Option<Uuid>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub path: String,
    /// Unix seconds.
    pub created_at: i64,
    /// The publishing domain's popularity signal, never negative.
    #[serde(default)]
    pub score: f64,
    /// Kind-specific fields, always a JSON object.
    #[serde(default = "empty_object")]
    pub extra: serde_json::Value,
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

impl IndexDoc {
    pub fn from_search_doc(doc: SearchDoc) -> Self {
        let handle = doc
            .extra
            .get("handle")
            .and_then(|h| h.as_str())
            .map(|h| clean(h.trim_start_matches('@'), MAX_HANDLE_CHARS))
            .filter(|h| !h.is_empty());
        let mut tags: Vec<String> = Vec::new();
        for tag in &doc.tags {
            let tag = clean(tag.trim().trim_start_matches(['#', '＃']), MAX_TAG_CHARS);
            if !tag.is_empty() && !tags.contains(&tag) {
                tags.push(tag);
            }
            if tags.len() == MAX_TAGS {
                break;
            }
        }
        Self {
            id: doc.id,
            kind: doc.kind,
            title: clean(&doc.title, MAX_TITLE_CHARS),
            handle,
            text: clean(&doc.text, MAX_TEXT_CHARS),
            tags,
            owner_id: doc.owner_id,
            image_url: doc.image_url.map(|url| clean(&url, MAX_URL_CHARS)).filter(|url| !url.is_empty()),
            path: clean(&doc.path, MAX_PATH_CHARS),
            created_at: doc.created_at.max(0),
            score: if doc.score.is_finite() { doc.score.max(0.0) } else { 0.0 },
            extra: if doc.extra.is_object() { doc.extra } else { empty_object() },
        }
    }

    /// The JSON written to Meilisearch: the document plus its `rank`.
    pub fn to_index_json(&self, kind: &Kind) -> serde_json::Value {
        let mut json = serde_json::to_value(self).expect("index documents serialize");
        json["rank"] = serde_json::json!(rank(kind, self.score, self.created_at));
        json
    }
}

/// The static ranking signal that orders equally relevant documents: a blend
/// of popularity and recency in the manner of a "hot" rank.
///
/// `log10(1 + score)` plus the document's age position on the kind's time
/// scale. One unit is a tenfold difference in popularity, and equally a
/// difference of `decade_days` in age: on the 90-day scale a video needs ten
/// times the views to hold its place against one published a quarter later.
/// The value never has to be recomputed as time passes, because only the
/// order between documents matters.
pub fn rank(kind: &Kind, score: f64, created_at: i64) -> f64 {
    let popularity = (1.0 + score.max(0.0)).log10();
    let recency = (created_at - RANK_EPOCH) as f64 / (kind.decade_days * 86_400.0);
    popularity + recency
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        kinds,
        text::{HIGHLIGHT_END, HIGHLIGHT_START},
    };

    fn doc() -> SearchDoc {
        SearchDoc {
            kind: "video".into(),
            id: Uuid::now_v7(),
            title: "  渋谷の\n夜  ".into(),
            text: "東京の渋谷を\r\n散歩する".into(),
            tags: vec!["#散歩".into(), "散歩".into(), " ".into(), "＃東京".into()],
            owner_id: Some(Uuid::now_v7()),
            public: true,
            image_url: Some("https://cdn.example/t.webp".into()),
            path: "/watch/abc".into(),
            created_at: 1_750_000_000,
            score: 12.0,
            extra: serde_json::json!({ "duration_seconds": 61 }),
        }
    }

    #[test]
    fn documents_are_cleaned_and_tags_deduplicated() {
        let indexed = IndexDoc::from_search_doc(doc());
        assert_eq!(indexed.title, "渋谷の 夜");
        assert_eq!(indexed.text, "東京の渋谷を 散歩する");
        assert_eq!(indexed.tags, vec!["散歩", "東京"]);
        assert_eq!(indexed.handle, None);
        assert_eq!(indexed.extra["duration_seconds"], 61);
    }

    #[test]
    fn a_handle_in_extra_becomes_searchable() {
        let mut source = doc();
        source.extra = serde_json::json!({ "handle": "@toku_zou", "verified": true });
        let indexed = IndexDoc::from_search_doc(source);
        assert_eq!(indexed.handle.as_deref(), Some("toku_zou"));
        assert_eq!(indexed.extra["handle"], "@toku_zou");
    }

    #[test]
    fn hostile_values_are_neutralized() {
        let mut source = doc();
        source.title = format!("{HIGHLIGHT_START}fake{HIGHLIGHT_END}");
        source.score = f64::NAN;
        source.created_at = -5;
        source.extra = serde_json::json!("not an object");
        source.image_url = Some("  ".into());
        let indexed = IndexDoc::from_search_doc(source);
        assert_eq!(indexed.title, "fake");
        assert_eq!(indexed.score, 0.0);
        assert_eq!(indexed.created_at, 0);
        assert_eq!(indexed.extra, serde_json::json!({}));
        assert_eq!(indexed.image_url, None);
    }

    #[test]
    fn long_fields_are_bounded() {
        let mut source = doc();
        source.title = "あ".repeat(500);
        source.text = "い".repeat(20_000);
        source.tags = (0..80).map(|n| format!("tag{n}")).collect();
        let indexed = IndexDoc::from_search_doc(source);
        assert_eq!(indexed.title.chars().count(), MAX_TITLE_CHARS);
        assert_eq!(indexed.text.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(indexed.tags.len(), MAX_TAGS);
    }

    #[test]
    fn index_json_carries_the_rank_and_survives_a_round_trip() {
        let indexed = IndexDoc::from_search_doc(doc());
        let kind = kinds::by_name("video").unwrap();
        let json = indexed.to_index_json(kind);
        assert_eq!(json["id"], indexed.id.to_string());
        assert_eq!(json["rank"].as_f64().unwrap(), rank(kind, 12.0, 1_750_000_000));
        let stored = serde_json::to_value(&indexed).unwrap();
        assert_eq!(serde_json::from_value::<IndexDoc>(stored).unwrap(), indexed);
    }

    #[test]
    fn rank_trades_popularity_against_age_on_the_kinds_time_scale() {
        let note = kinds::by_name("note").unwrap();
        let day = 86_400;
        let now = 1_760_000_000;
        // Newer wins at equal popularity; more popular wins at equal age.
        assert!(rank(note, 10.0, now) > rank(note, 10.0, now - day));
        assert!(rank(note, 100.0, now) > rank(note, 10.0, now));
        // Ten times the popularity outweighs a little less than one time scale of age...
        let scale = note.decade_days as i64;
        assert!(rank(note, 999.0, now - (scale - 1) * day) > rank(note, 99.0, now));
        // ...and not more than that.
        assert!(rank(note, 999.0, now - (scale + 1) * day) < rank(note, 99.0, now));
        // Kinds on the same scale rank alike, so they can be merged.
        let video = kinds::by_name("video").unwrap();
        assert_eq!(rank(note, 42.0, now - 7 * day), rank(video, 42.0, now - 7 * day));
        // Accounts barely age.
        let user = kinds::by_name("user").unwrap();
        assert!(rank(user, 20.0, now - 365 * day) > rank(user, 10.0, now));
    }
}
