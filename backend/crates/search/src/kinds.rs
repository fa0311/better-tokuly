//! The kinds of searchable documents.
//!
//! A kind is the unit of everything here: it names the Meilisearch index
//! (`<prefix>_<kind>`), the tab in the API (`SearchKind`) and the value of
//! `tokuly_core::outbound::SearchDoc::kind` the owning domain publishes.
use tokuly_proto::search::SearchKind;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kind {
    /// Name in `SearchDoc::kind`, in index names and in the registry.
    pub name: &'static str,
    pub proto: SearchKind,
    /// How quickly this kind of content goes stale: the age, in days, that
    /// costs a document as much rank as a tenfold drop in popularity (see
    /// [`crate::document::rank`]). One of two scales, [`TIMELY`] or
    /// [`EVERGREEN`].
    pub decade_days: f64,
    /// Weight of this kind's matches when all kinds are searched at once.
    /// Slightly above 1 lets a kind win near-ties (people before posts).
    pub weight: f32,
    /// Extra weight in typeahead, where the user is usually after a name.
    pub typeahead_weight: f32,
}

/// Time scale of content that is about a moment: posts, videos, streams,
/// news, answers, threads, audio rooms. Each quarter of age costs as much as a tenfold drop in popularity.
///
/// There are only two scales, and kinds on the same scale age at the same
/// rate, on purpose: `rank` is also what orders equally relevant documents of
/// *different* kinds in the "all" tab, and ranks are only comparable between
/// kinds that count time the same way. (Between the two groups a tie goes to
/// the timely document.)
pub const TIMELY: f64 = 90.0;
/// Time scale of things that simply exist: accounts, channels, places,
/// hashtags, playlists, stamp packs, emoji, communities. Age hardly matters, popularity decides.
pub const EVERGREEN: f64 = 3650.0;

/// Every kind, in `SearchKind` order.
pub const KINDS: &[Kind] = &[
    Kind {
        name: "user",
        proto: SearchKind::SEARCH_KIND_USER,
        decade_days: EVERGREEN,
        weight: 1.05,
        typeahead_weight: 1.15,
    },
    Kind {
        name: "channel",
        proto: SearchKind::SEARCH_KIND_CHANNEL,
        decade_days: EVERGREEN,
        weight: 1.04,
        typeahead_weight: 1.1,
    },
    Kind { name: "note", proto: SearchKind::SEARCH_KIND_NOTE, decade_days: TIMELY, weight: 1.0, typeahead_weight: 1.0 },
    Kind {
        name: "video",
        proto: SearchKind::SEARCH_KIND_VIDEO,
        decade_days: TIMELY,
        weight: 1.0,
        typeahead_weight: 1.0,
    },
    Kind {
        name: "short",
        proto: SearchKind::SEARCH_KIND_SHORT,
        decade_days: TIMELY,
        weight: 1.0,
        typeahead_weight: 1.0,
    },
    Kind { name: "live", proto: SearchKind::SEARCH_KIND_LIVE, decade_days: TIMELY, weight: 1.0, typeahead_weight: 1.0 },
    Kind { name: "clip", proto: SearchKind::SEARCH_KIND_CLIP, decade_days: TIMELY, weight: 1.0, typeahead_weight: 1.0 },
    Kind {
        name: "playlist",
        proto: SearchKind::SEARCH_KIND_PLAYLIST,
        decade_days: EVERGREEN,
        weight: 1.0,
        typeahead_weight: 1.0,
    },
    Kind {
        name: "stamp_pack",
        proto: SearchKind::SEARCH_KIND_STAMP_PACK,
        decade_days: EVERGREEN,
        weight: 1.0,
        typeahead_weight: 1.0,
    },
    Kind {
        name: "place",
        proto: SearchKind::SEARCH_KIND_PLACE,
        decade_days: EVERGREEN,
        weight: 1.02,
        typeahead_weight: 1.05,
    },
    Kind {
        name: "hashtag",
        proto: SearchKind::SEARCH_KIND_HASHTAG,
        decade_days: EVERGREEN,
        weight: 1.02,
        typeahead_weight: 1.05,
    },
    Kind { name: "news", proto: SearchKind::SEARCH_KIND_NEWS, decade_days: TIMELY, weight: 1.0, typeahead_weight: 1.0 },
    Kind {
        name: "answer",
        proto: SearchKind::SEARCH_KIND_ANSWER,
        decade_days: TIMELY,
        weight: 1.0,
        typeahead_weight: 1.0,
    },
    Kind {
        name: "emoji",
        proto: SearchKind::SEARCH_KIND_EMOJI,
        decade_days: EVERGREEN,
        weight: 1.0,
        typeahead_weight: 1.0,
    },
    Kind {
        name: "community",
        proto: SearchKind::SEARCH_KIND_COMMUNITY,
        decade_days: EVERGREEN,
        weight: 1.03,
        typeahead_weight: 1.08,
    },
    Kind {
        name: "thread",
        proto: SearchKind::SEARCH_KIND_THREAD,
        decade_days: TIMELY,
        weight: 1.0,
        typeahead_weight: 1.0,
    },
    Kind {
        name: "space",
        proto: SearchKind::SEARCH_KIND_SPACE,
        decade_days: TIMELY,
        weight: 1.0,
        typeahead_weight: 1.0,
    },
];

/// The kind users are indexed under (by this crate, see [`crate::users`]).
pub const USER: &str = "user";

/// Kinds published on a channel. A channel does not reveal the account
/// behind it (`tokuly.video.v1`), so their documents keep `owner_id` only for
/// blocks, suspensions and deletion: a hit never names the owner, and
/// searching one account's content leaves them out.
const CHANNEL_KINDS: &[&str] = &["channel", "video", "short", "live"];

/// Whether a hit of `kind` must not name its owner: channel content, and a
/// playlist published on a channel (`extra.channel_id` set).
pub fn hides_owner(kind: &str, extra: &serde_json::Value) -> bool {
    CHANNEL_KINDS.contains(&kind) || (kind == "playlist" && extra.get("channel_id").is_some_and(|id| id.is_string()))
}

/// Whether someone other than the owner may search one account's documents
/// of `kind` ("search this profile"). Not for channel content, which would
/// list the channels of an account; nor for playlists, whose channel is not
/// something the engine can filter on.
pub fn searchable_by_owner(kind: &str) -> bool {
    !CHANNEL_KINDS.contains(&kind) && kind != "playlist"
}

pub fn by_name(name: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|kind| kind.name == name)
}

pub fn by_proto(proto: SearchKind) -> Option<&'static Kind> {
    KINDS.iter().find(|kind| kind.proto == proto)
}

/// The kind a moderated subject is indexed under, when it is indexed at all.
pub fn by_subject_type(subject_type: i32) -> Option<&'static Kind> {
    use tokuly_proto::common::SubjectType as S;
    let name = match buffa::EnumValue::<S>::from(subject_type).as_known()? {
        S::SUBJECT_TYPE_USER => "user",
        S::SUBJECT_TYPE_CHANNEL => "channel",
        S::SUBJECT_TYPE_NOTE => "note",
        S::SUBJECT_TYPE_VIDEO => "video",
        S::SUBJECT_TYPE_SHORT => "short",
        S::SUBJECT_TYPE_LIVE_STREAM => "live",
        S::SUBJECT_TYPE_CLIP => "clip",
        S::SUBJECT_TYPE_PLAYLIST => "playlist",
        S::SUBJECT_TYPE_STAMP_PACK => "stamp_pack",
        S::SUBJECT_TYPE_PLACE => "place",
        S::SUBJECT_TYPE_NEWS_ARTICLE => "news",
        // An answer is indexed under the ID of its question.
        S::SUBJECT_TYPE_QUESTION => "answer",
        S::SUBJECT_TYPE_CUSTOM_EMOJI => "emoji",
        S::SUBJECT_TYPE_COMMUNITY => "community",
        S::SUBJECT_TYPE_COMMUNITY_THREAD => "thread",
        // Rooms; a hangout is the same subject type and has no document.
        S::SUBJECT_TYPE_SPACE => "space",
        _ => return None,
    };
    by_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_api_kind_has_exactly_one_spec() {
        assert_eq!(KINDS.len(), 17);
        for (position, kind) in KINDS.iter().enumerate() {
            assert_eq!(kind.proto as i32, position as i32 + 1, "{} is out of SearchKind order", kind.name);
            assert_eq!(by_name(kind.name), Some(kind));
            assert_eq!(by_proto(kind.proto), Some(kind));
            assert!(
                kind.decade_days == TIMELY || kind.decade_days == EVERGREEN,
                "{} needs a shared time scale",
                kind.name
            );
            assert!(kind.weight > 0.0 && kind.typeahead_weight > 0.0);
        }
        assert!(by_proto(SearchKind::SEARCH_KIND_UNSPECIFIED).is_none());
        assert!(by_name("comment").is_none());
    }

    #[test]
    fn channel_content_hides_its_owner() {
        let none = serde_json::json!({});
        for kind in ["channel", "video", "short", "live"] {
            assert!(hides_owner(kind, &none) && !searchable_by_owner(kind), "{kind}");
        }
        assert!(hides_owner("playlist", &serde_json::json!({ "channel_id": "c" })));
        assert!(!hides_owner("playlist", &serde_json::json!({ "channel_id": null })));
        assert!(!searchable_by_owner("playlist"));
        for kind in ["user", "note", "answer", "clip", "stamp_pack", "thread"] {
            assert!(!hides_owner(kind, &none) && searchable_by_owner(kind), "{kind}");
        }
    }

    #[test]
    fn moderated_subjects_map_to_their_index() {
        use tokuly_proto::common::SubjectType as S;
        assert_eq!(by_subject_type(S::SUBJECT_TYPE_LIVE_STREAM as i32).unwrap().name, "live");
        assert_eq!(by_subject_type(S::SUBJECT_TYPE_NEWS_ARTICLE as i32).unwrap().name, "news");
        assert_eq!(by_subject_type(S::SUBJECT_TYPE_STAMP_PACK as i32).unwrap().name, "stamp_pack");
        assert_eq!(by_subject_type(S::SUBJECT_TYPE_QUESTION as i32).unwrap().name, "answer");
        assert_eq!(by_subject_type(S::SUBJECT_TYPE_CUSTOM_EMOJI as i32).unwrap().name, "emoji");
        assert_eq!(by_subject_type(S::SUBJECT_TYPE_COMMUNITY as i32).unwrap().name, "community");
        assert_eq!(by_subject_type(S::SUBJECT_TYPE_COMMUNITY_THREAD as i32).unwrap().name, "thread");
        assert_eq!(by_subject_type(S::SUBJECT_TYPE_SPACE as i32).unwrap().name, "space");
        // Not indexed.
        assert!(by_subject_type(S::SUBJECT_TYPE_COMMENT as i32).is_none());
        assert!(by_subject_type(S::SUBJECT_TYPE_STORY as i32).is_none());
        assert!(by_subject_type(S::SUBJECT_TYPE_COMMUNITY_REPLY as i32).is_none());
        assert!(by_subject_type(9999).is_none());
    }
}
