//! From an event to credits: the hot path.
//!
//! [`Quest::handle_event`] (domain events) and [`Quest::handle_analytics`]
//! (analytics events) look the event up in the rule table, let the rule's
//! reader say who did what, and credit each deed to that player's day in
//! Valkey. No PostgreSQL is touched for a deed, with two exceptions that
//! happen once per player (and friend) and day rather than once per event:
//! loading a day's counters the first time they are needed, and recording a
//! player's part of a streak day.
//!
//! Everything here can be repeated: a deed is applied once per event ID
//! (events are redelivered after failures), and the streak machine ignores
//! parts it already has. An event the table does not know, or whose payload
//! cannot be read, is a no-op.
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokuly_core::{analytics::AnalyticsEvent, events::Event, prelude::*};
use tokuly_social::graph;

use crate::{
    Quest, clock,
    metrics::Metric,
    payloads::{self, Deed, Target},
    rules::{self, Feed, Rule, Source},
    store::{Credit, SEEN_TTL_SECONDS},
};

pub(crate) const USER_DELETED: &str = "identity.user.deleted";
const FRIEND_REMOVED: &str = "social.friend.removed";
const CONVERSATION_CREATED: &str = "chat.conversation.created";

/// `tokuly.chat.v1.ConversationKind.CONVERSATION_KIND_DIRECT`.
const CONVERSATION_KIND_DIRECT: i32 = 1;

/// How often one process re-records that a rule is in use.
const LIVE_MARK_EVERY: Duration = Duration::from_secs(600);

/// The domain event topics the quest consumer listens to: every topic of
/// the rule table, plus the ones it keeps its own house with.
pub(crate) fn event_topics() -> Vec<&'static str> {
    let mut topics = rules::topics(Feed::Events);
    topics.extend([USER_DELETED, FRIEND_REMOVED, CONVERSATION_CREATED]);
    topics.sort_unstable();
    topics.dedup();
    topics
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UserDeleted {
    user_id: Option<Uuid>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FriendRemoved {
    user_id: Option<Uuid>,
    friend_id: Option<Uuid>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ConversationCreated {
    conversation_id: Option<Uuid>,
    kind: i32,
    member_ids: Vec<Uuid>,
}

/// Reads a housekeeping payload; a payload that cannot be read is logged and skipped.
fn read<T: serde::de::DeserializeOwned>(event: &Event) -> Option<T> {
    match T::deserialize(&event.data) {
        Ok(payload) => Some(payload),
        Err(e) => {
            tracing::warn!(topic = %event.topic, id = %event.id, error = %e, "quest: event payload not understood; ignored");
            None
        }
    }
}

impl Quest {
    /// Handles one domain event. Events are delivered at least once, so
    /// everything behind this is safe to repeat.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => {
                if let Some(UserDeleted { user_id: Some(user_id) }) = read(&event) {
                    self.erase_player(user_id).await?;
                }
                return Ok(());
            }
            FRIEND_REMOVED => {
                if let Some(FriendRemoved { user_id: Some(user_id), friend_id: Some(friend_id) }) = read(&event) {
                    self.end_streak(user_id, friend_id, event.occurred_at).await?;
                }
                return Ok(());
            }
            CONVERSATION_CREATED => {
                if let Some(created) = read::<ConversationCreated>(&event) {
                    self.remember_direct_chat(created).await?;
                }
                return Ok(());
            }
            _ => {}
        }
        self.apply_rules(Feed::Events, &event).await
    }

    /// Handles one analytics event (the feed for things that happen too
    /// often to be domain events: chat messages, stamp sends, likes).
    pub async fn handle_analytics(&self, tracked: AnalyticsEvent) -> AppResult<()> {
        self.apply_rules(Feed::Analytics, &payloads::analytics_as_event(tracked)).await
    }

    async fn apply_rules(&self, feed: Feed, event: &Event) -> AppResult<()> {
        let now = Utc::now();
        // Past this age a deed could no longer be recognised as already
        // applied, so it is not applied at all.
        if now - event.occurred_at > chrono::Duration::seconds(SEEN_TTL_SECONDS - 3600) {
            tracing::warn!(topic = %event.topic, id = %event.id, occurred_at = %event.occurred_at, "quest: event too old to count; ignored");
            return Ok(());
        }
        // A producer's clock running ahead must not open tomorrow early.
        let at = event.occurred_at.min(now);
        let kind = event.data.get("kind").and_then(|kind| kind.as_str()).unwrap_or_default();
        for rule in rules::rules_for(feed, &event.topic) {
            if !rule.kind.is_empty() && rule.kind != kind {
                continue;
            }
            let deeds = (rule.read)(event);
            if deeds.is_empty() {
                continue;
            }
            self.mark_live(rule).await;
            for deed in &deeds {
                self.credit_deed(rule, deed, event, at).await?;
            }
        }
        Ok(())
    }

    /// Credits one deed to its doer's day, and — when the rule is something
    /// done towards a friend — records the doer's part of the streak day
    /// with each friend it was aimed at.
    async fn credit_deed(&self, rule: &'static Rule, deed: &Deed, event: &Event, at: DateTime<Utc>) -> AppResult<()> {
        // Accounts that are gone (or banned) earn nothing.
        let Some(account) = self.account_of(deed.user).await? else { return Ok(()) };
        // Nor does where somebody has been earn anything while they keep it to themselves.
        if rule.location && account.hides_footprints {
            return Ok(());
        }
        let zone = account.zone;
        let day = clock::local_date(at, zone);
        let hour = clock::local_hour(at, zone);
        let counts: Vec<(Metric, i64)> =
            rule.counts.iter().map(|count| (count.metric(), count.for_amount(deed.amount))).collect();
        let dedupe = format!("{}:{}:{}", event.id, rule.key, deed.user);
        self.credit(&Credit {
            user: deed.user,
            day,
            hour,
            dedupe: &dedupe,
            source: rule.source,
            xp: rule.xp.for_amount(deed.amount),
            counts: &counts,
        })
        .await?;
        // Also when the credit was a repeat: an earlier delivery may have
        // stopped between the credit and the streak.
        if rule.streak {
            for target in &deed.towards {
                let Some(friend) = self.resolve_target(deed.user, *target).await? else { continue };
                if self.towards_marker(deed.user, friend, day).await?.is_some() {
                    continue; // today's part is recorded, or the two are known not to be friends
                }
                if !graph::are_friends(&self.core, deed.user, friend).await? {
                    self.mark_towards(deed.user, friend, day, false).await?;
                    continue;
                }
                self.record_part(deed.user, friend, at).await?;
                if self.mark_towards(deed.user, friend, day, true).await? {
                    let dedupe = format!("reach:{}:{friend}:{}", deed.user, clock::date_text(day));
                    self.credit(&Credit {
                        user: deed.user,
                        day,
                        hour,
                        dedupe: &dedupe,
                        source: Source::Friends,
                        xp: 0,
                        counts: &[(Metric::FriendsReached, 1)],
                    })
                    .await?;
                }
            }
        }
        Ok(())
    }

    /// The user a deed was aimed at, if that can be told and is somebody else.
    async fn resolve_target(&self, doer: Uuid, target: Target) -> AppResult<Option<Uuid>> {
        let other = match target {
            Target::User(other) => Some(other),
            Target::DirectChat(conversation) => self.direct_chat(conversation).await?.and_then(|(lo, hi)| {
                if doer == lo {
                    Some(hi)
                } else if doer == hi {
                    Some(lo)
                } else {
                    None
                }
            }),
        };
        Ok(other.filter(|other| *other != doer && !other.is_nil()))
    }

    /// The two members of a direct conversation, when it is a known one.
    async fn direct_chat(&self, conversation: Uuid) -> AppResult<Option<(Uuid, Uuid)>> {
        if let Some(members) = self.direct_chats.get(&conversation).await {
            return Ok(Some(members));
        }
        if self.unknown_chats.get(&conversation).await.is_some() {
            return Ok(None);
        }
        let mut members: Option<(Uuid, Uuid)> =
            sqlx::query_as("SELECT user_lo, user_hi FROM quest_direct_chats WHERE conversation_id = $1")
                .bind(conversation)
                .fetch_optional(&self.core.db)
                .await?;
        // A conversation from before this domain listened: the chat domain knows, if the server connected the two.
        if members.is_none()
            && let Some(lookup) = self.direct_chat_lookup()
            && let Some((a, b)) = lookup.members(conversation).await?
        {
            self.remember_direct_chat(ConversationCreated {
                conversation_id: Some(conversation),
                kind: CONVERSATION_KIND_DIRECT,
                member_ids: vec![a, b],
            })
            .await?;
            members = Some(if a < b { (a, b) } else { (b, a) });
        }
        match members {
            Some(members) => self.direct_chats.insert(conversation, members).await,
            // Briefly, so a busy conversation from before this domain existed
            // (or a group reported without its flag) costs one lookup a minute.
            None => self.unknown_chats.insert(conversation, ()).await,
        }
        Ok(members)
    }

    /// `chat.conversation.created`: remembers who the two members of a
    /// direct conversation are (chat announces messages without a recipient).
    async fn remember_direct_chat(&self, created: ConversationCreated) -> AppResult<()> {
        let (Some(conversation), [a, b]) = (created.conversation_id, created.member_ids.as_slice()) else {
            return Ok(());
        };
        if created.kind != CONVERSATION_KIND_DIRECT || a == b {
            return Ok(());
        }
        let (lo, hi) = if a < b { (*a, *b) } else { (*b, *a) };
        sqlx::query(
            "INSERT INTO quest_direct_chats (conversation_id, user_lo, user_hi)
             SELECT $1, $2, $3 WHERE (SELECT count(*) FROM users WHERE id IN ($2, $3)) = 2
             ON CONFLICT (conversation_id) DO NOTHING",
        )
        .bind(conversation)
        .bind(lo)
        .bind(hi)
        .execute(&self.core.db)
        .await?;
        self.unknown_chats.invalidate(&conversation).await;
        Ok(())
    }

    /// Records that a rule is in use on this server (at most every ten
    /// minutes per process). Best effort: it only decides which missions
    /// and boards are offered.
    async fn mark_live(&self, rule: &'static Rule) {
        {
            let mut marks = self.rule_marks.lock();
            if marks.get(rule.key).is_some_and(|marked| marked.elapsed() < LIVE_MARK_EVERY) {
                return;
            }
            marks.insert(rule.key, Instant::now());
        }
        let stored = sqlx::query("INSERT INTO quest_rule_activity (rule, last_seen_at) VALUES ($1, now()) ON CONFLICT (rule) DO UPDATE SET last_seen_at = EXCLUDED.last_seen_at")
            .bind(rule.key)
            .execute(&self.core.db)
            .await;
        match stored {
            Ok(_) => self.live_rules.invalidate(&()).await,
            Err(e) => {
                tracing::warn!(rule = rule.key, error = %e, "quest: rule activity not recorded");
                self.rule_marks.lock().remove(rule.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_consumer_listens_to_the_rules_and_its_housekeeping() {
        let topics = event_topics();
        for topic in [
            USER_DELETED,
            FRIEND_REMOVED,
            CONVERSATION_CREATED,
            "social.bump.created",
            "notify.requested",
            "activity.recorded",
            "school.session.completed",
        ] {
            assert!(topics.contains(&topic), "{topic} is not consumed");
        }
        // Analytics names are consumed from the other stream.
        assert!(!topics.contains(&"chat.message.send"));
        let mut unique = topics.clone();
        unique.dedup();
        assert_eq!(unique.len(), topics.len());
    }
}
