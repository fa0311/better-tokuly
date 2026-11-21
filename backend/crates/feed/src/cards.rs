//! Feed cards: what the friends' feed and the home feed are made of.
//!
//! Both feeds show other people's entries newest first, and both collapse a
//! *burst* — entries of the same kind by the same person, each no further
//! than the burst window from the next — into one card. A burst is
//! identified by its newest entry, the *head*: an entry that has no newer
//! entry of the same person and kind within the window. Pages are keyset
//! pages over heads ([`Feed::card_heads`]); the older entries of each burst
//! are loaded afterwards ([`Feed::bursts`]).
//!
//! The home feed adds three things on top (see [`CardQuery::home`]): only
//! entries with a card type, no second card about the same piece of content,
//! and one card instead of two when both people of a pairwise event (a bump,
//! a new friendship) are friends of the reader.
use std::collections::{HashMap, VecDeque};

use chrono::Duration;
use sqlx::{Postgres, QueryBuilder};
use tokuly_core::prelude::*;
use tokuly_proto::{common::SubjectType, feed::ActivityCard};

use crate::{
    Feed,
    activity::{AUDIENCE_FRIENDS, ActivityRow, Reader, activity_columns, author_active, kind_shown, push_kind_filter, push_not_involving, to_activity},
    kinds,
};

/// Most older entries loaded for one burst; a longer burst shows this many.
const BURST_MAX_MEMBERS: i64 = 50;
/// How far back from its head a burst may reach.
const BURST_MAX_SPAN_HOURS: i32 = 24;
/// Older entries a card carries besides its head.
const CARD_MORE: usize = 4;
/// How long a newer card about the same content suppresses an older one.
const SAME_CONTENT_DAYS: i32 = 7;
/// How far apart the two entries of a pairwise event may be recorded.
const PAIR_MINUTES: i32 = 10;
/// How many cards ahead the home feed looks for one by somebody else.
const SPREAD_LOOKAHEAD: usize = 8;

/// One page of cards to select.
pub(crate) struct CardQuery<'a> {
    /// Whose entries: the reader's friends.
    pub authors: &'a [Uuid],
    /// Everyone hidden from the reader by a block.
    pub hidden: &'a [Uuid],
    pub kind_prefix: Option<&'a str>,
    /// Keyset of the last head of the previous page.
    pub after: Option<(Time, Uuid)>,
    pub limit: i64,
    /// Home feed rules: typed cards only, de-duplicated.
    pub home: bool,
}

/// A head with the older entries collapsed into it, newest first.
pub(crate) struct Burst {
    pub head: ActivityRow,
    pub older: Vec<ActivityRow>,
}

#[derive(sqlx::FromRow)]
struct MemberRow {
    head_id: Uuid,
    #[sqlx(flatten)]
    row: ActivityRow,
}

impl Feed {
    fn burst_window(&self) -> Duration {
        Duration::minutes(i64::from(self.cfg.burst_minutes))
    }

    /// The next `limit` burst heads visible to the reader, newest first.
    pub(crate) async fn card_heads(&self, q: &CardQuery<'_>) -> AppResult<Vec<ActivityRow>> {
        if q.authors.is_empty() {
            return Ok(Vec::new());
        }
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(concat!("SELECT ", activity_columns!(), " FROM activities a WHERE a.user_id = ANY("));
        query.push_bind(q.authors.to_vec());
        query.push(") AND a.audience >= ").push_bind(AUDIENCE_FRIENDS);
        query.push(concat!(" AND ", author_active!("a"), " AND ", kind_shown!("a")));
        push_not_involving(&mut query, "a", q.hidden);
        push_kind_filter(&mut query, "a", q.kind_prefix);
        if let Some((at, id)) = q.after {
            query.push(" AND (a.occurred_at, a.id) < (").push_bind(at).push(", ").push_bind(id).push(")");
        }
        if self.cfg.burst_minutes > 0 {
            // Not a head: a newer entry of the same person and kind follows within the window.
            query.push(
                " AND NOT EXISTS (SELECT 1 FROM activities n WHERE n.user_id = a.user_id AND n.kind = a.kind AND n.audience >= 2
                      AND (n.occurred_at, n.id) > (a.occurred_at, a.id) AND n.occurred_at <= a.occurred_at + make_interval(mins => ",
            );
            query.push_bind(self.cfg.burst_minutes as i32).push(")");
            push_not_involving(&mut query, "n", q.hidden);
            query.push(")");
        }
        if q.home {
            self.push_home_rules(&mut query, q);
        }
        query.push(" ORDER BY a.occurred_at DESC, a.id DESC LIMIT ").push_bind(q.limit);
        Ok(query.build_query_as().fetch_all(&self.core.db).await?)
    }

    /// The home feed's extra conditions on a head `a`.
    fn push_home_rules(&self, query: &mut QueryBuilder<Postgres>, q: &CardQuery<'_>) {
        // Only entries with a card type (`kinds::card_type` as data).
        query.push(" AND (COALESCE(a.subject_type, 0) = ANY(").push_bind(kinds::card_subject_types());
        query.push(") OR EXISTS (SELECT 1 FROM unnest(").push_bind(kinds::card_kind_prefixes());
        query.push("::text[]) AS p WHERE a.kind = p OR starts_with(a.kind, p || '.')))");

        // The same piece of content already has a newer card.
        query.push(" AND NOT (COALESCE(a.subject_type, 0) = ANY(").push_bind(kinds::content_subject_types());
        query.push(
            ") AND EXISTS (SELECT 1 FROM activities d WHERE d.subject_type = a.subject_type AND d.subject_id = a.subject_id
                   AND (d.occurred_at, d.id) > (a.occurred_at, a.id) AND d.audience >= 2 AND d.user_id = ANY(",
        );
        query.push_bind(q.authors.to_vec());
        query.push(") AND d.occurred_at <= a.occurred_at + make_interval(days => ").push_bind(SAME_CONTENT_DAYS).push(")");
        query.push(concat!(" AND ", kind_shown!("d")));
        push_not_involving(query, "d", q.hidden);
        query.push("))");

        // A pairwise event recorded by both people: keep the entry of the
        // person with the smaller ID.
        let user_subject = SubjectType::SUBJECT_TYPE_USER as i32;
        query.push(format!(
            " AND NOT (COALESCE(a.subject_type, 0) = {user_subject} AND EXISTS (SELECT 1 FROM activities d
                   WHERE d.user_id = a.subject_id AND d.kind = a.kind AND d.subject_type = {user_subject} AND d.subject_id = a.user_id
                     AND d.user_id < a.user_id AND d.audience >= 2 AND d.user_id = ANY("
        ));
        query.push_bind(q.authors.to_vec());
        query.push(") AND d.occurred_at BETWEEN a.occurred_at - make_interval(mins => ").push_bind(PAIR_MINUTES);
        query.push(") AND a.occurred_at + make_interval(mins => ").push_bind(PAIR_MINUTES).push(")");
        query.push(concat!(" AND ", kind_shown!("d"), "))"));
    }

    /// Loads the older entries of each head's burst.
    pub(crate) async fn bursts(&self, heads: Vec<ActivityRow>, hidden: &[Uuid]) -> AppResult<Vec<Burst>> {
        if heads.is_empty() || self.cfg.burst_minutes == 0 {
            return Ok(heads.into_iter().map(|head| Burst { head, older: Vec::new() }).collect());
        }
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(concat!(
            "SELECT h.id AS head_id, ",
            activity_columns!(),
            " FROM activities h CROSS JOIN LATERAL (
                 SELECT m.* FROM activities m
                 WHERE m.user_id = h.user_id AND m.kind = h.kind AND m.audience >= 2
                   AND (m.occurred_at, m.id) < (h.occurred_at, h.id)
                   AND m.occurred_at >= h.occurred_at - make_interval(hours => "
        ));
        query.push_bind(BURST_MAX_SPAN_HOURS).push(")");
        push_not_involving(&mut query, "m", hidden);
        query.push(" ORDER BY m.occurred_at DESC, m.id DESC LIMIT ").push_bind(BURST_MAX_MEMBERS);
        query.push(") a WHERE h.id = ANY(").push_bind(heads.iter().map(|head| head.id).collect::<Vec<_>>());
        query.push(") ORDER BY h.id, a.occurred_at DESC, a.id DESC");
        let members: Vec<MemberRow> = query.build_query_as().fetch_all(&self.core.db).await?;

        let mut by_head: HashMap<Uuid, Vec<ActivityRow>> = HashMap::new();
        for member in members {
            by_head.entry(member.head_id).or_default().push(member.row);
        }
        let window = self.burst_window();
        Ok(heads
            .into_iter()
            .map(|head| {
                let older = chain(head.occurred_at, by_head.remove(&head.id).unwrap_or_default(), window);
                Burst { head, older }
            })
            .collect())
    }

    /// Bursts as API cards, in the reader's language.
    pub(crate) async fn to_cards(&self, reader: &Reader, bursts: &[Burst]) -> AppResult<Vec<ActivityCard>> {
        let shown: Vec<&ActivityRow> = bursts.iter().flat_map(|burst| std::iter::once(&burst.head).chain(burst.older.iter().take(CARD_MORE))).collect();
        let people = self.people_of(&shown).await?;
        Ok(bursts
            .iter()
            .map(|burst| ActivityCard {
                r#type: kinds::card_type(&burst.head.kind, burst.head.subject_type).into(),
                activity: to_activity(&burst.head, reader, &people, None).into(),
                count: 1 + burst.older.len() as i32,
                more: burst.older.iter().take(CARD_MORE).map(|row| to_activity(row, reader, &people, None)).collect(),
                ..Default::default()
            })
            .collect())
    }
}

/// The older entries that belong to a burst: walking back from the head,
/// every entry no further than `window` from the one after it. `candidates`
/// are the same person's older entries of the same kind, newest first.
pub(crate) fn chain(head_at: Time, candidates: Vec<ActivityRow>, window: Duration) -> Vec<ActivityRow> {
    let mut next_at = head_at;
    let mut members = Vec::new();
    for candidate in candidates {
        if next_at - candidate.occurred_at > window {
            break;
        }
        next_at = candidate.occurred_at;
        members.push(candidate);
    }
    members
}

/// Re-orders a page so that one person's cards do not come in a row while
/// somebody else's card is close by: each position takes the earliest of the
/// next few remaining items whose actor differs from the previous one, or
/// simply the earliest when there is none. The order is otherwise kept, and
/// nothing is dropped.
pub(crate) fn spread<T>(items: Vec<T>, actor: impl Fn(&T) -> Uuid) -> Vec<T> {
    let mut pending: VecDeque<T> = items.into();
    let mut arranged = Vec::with_capacity(pending.len());
    let mut previous: Option<Uuid> = None;
    while !pending.is_empty() {
        let pick = pending.iter().take(SPREAD_LOOKAHEAD).position(|item| Some(actor(item)) != previous).unwrap_or(0);
        let Some(item) = pending.remove(pick) else { break };
        previous = Some(actor(&item));
        arranged.push(item);
    }
    arranged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(at: Time) -> ActivityRow {
        ActivityRow {
            id: Uuid::now_v7(),
            user_id: Uuid::nil(),
            kind: "note.posted".into(),
            audience: AUDIENCE_FRIENDS,
            subject_type: None,
            subject_id: None,
            with_user_ids: Vec::new(),
            summary_ja: String::new(),
            summary_en: String::new(),
            path: String::new(),
            image_url: None,
            lat: None,
            lng: None,
            data: serde_json::Value::Null,
            occurred_at: at,
        }
    }

    #[test]
    fn a_burst_reaches_back_while_the_gaps_stay_within_the_window() {
        let head_at = now();
        let window = Duration::minutes(60);
        let minutes_before = |m: i64| row(head_at - Duration::minutes(m));
        // 50 and 100 minutes before the head are each within an hour of the
        // next newer entry; 170 is 70 minutes from the previous one.
        let members = chain(head_at, vec![minutes_before(50), minutes_before(100), minutes_before(170), minutes_before(180)], window);
        assert_eq!(members.len(), 2);
        assert_eq!(members[1].occurred_at, head_at - Duration::minutes(100));

        assert!(chain(head_at, vec![minutes_before(61)], window).is_empty());
        assert_eq!(chain(head_at, vec![minutes_before(60)], window).len(), 1, "the window is inclusive");
        assert!(chain(head_at, Vec::new(), window).is_empty());
    }

    #[test]
    fn spreading_breaks_up_runs_of_one_person() {
        let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        // (actor, original position)
        let page = vec![(a, 0), (a, 1), (a, 2), (a, 3), (b, 4), (c, 5), (a, 6), (b, 7)];
        let arranged = spread(page.clone(), |item| item.0);
        assert_eq!(arranged.len(), page.len(), "nothing is dropped");
        let actors: Vec<Uuid> = arranged.iter().map(|item| item.0).collect();
        assert_eq!(actors, vec![a, b, a, c, a, b, a, a]);
        // Each person's own cards stay in their original order.
        for person in [a, b, c] {
            let positions: Vec<i32> = arranged.iter().filter(|item| item.0 == person).map(|item| item.1).collect();
            assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{positions:?}");
        }
    }

    #[test]
    fn spreading_leaves_varied_and_single_person_pages_alone() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let varied = vec![(a, 0), (b, 1), (a, 2), (b, 3)];
        assert_eq!(spread(varied.clone(), |item| item.0), varied);
        let alone = vec![(a, 0), (a, 1), (a, 2)];
        assert_eq!(spread(alone.clone(), |item| item.0), alone);
        assert!(spread(Vec::<(Uuid, i32)>::new(), |item| item.0).is_empty());
    }

    #[test]
    fn spreading_only_looks_a_few_cards_ahead() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        // Somebody else's card far down the page is not pulled to the top.
        let mut page: Vec<(Uuid, usize)> = (0..SPREAD_LOOKAHEAD + 4).map(|i| (a, i)).collect();
        page.push((b, 99));
        let arranged = spread(page, |item| item.0);
        let position_of_b = arranged.iter().position(|item| item.0 == b).unwrap();
        assert!(position_of_b >= 4, "b moved up at most to where the lookahead first reaches it, got {position_of_b}");
    }
}
