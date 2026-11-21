//! Topic names and who may subscribe to them.
//!
//! A topic is `<kind>:<id>[:<id>...]`, for example `live:<stream_id>` or
//! `subject:video:<id>`. The gateway itself only checks the shape of the
//! name. Whether a given user (or an anonymous connection) may follow a
//! topic is knowledge of the domain owning that kind of topic, which
//! registers a guard in `core.topic_guards`; kinds without a guard are
//! public.
use tokuly_core::realtime::TopicGuards;
use uuid::Uuid;

use crate::frames::{Code, Rejection};

/// Longest accepted topic name, in bytes.
pub(crate) const MAX_TOPIC_LEN: usize = 128;

const INVALID: Rejection = Rejection::new(Code::InvalidArgument, "TOPIC_INVALID");

/// Asks the owning domain's guard whether `subscriber` (`None` = anonymous
/// connection) may follow a (valid) topic. The check runs once, when a
/// connection subscribes.
pub(crate) async fn authorize(guards: &TopicGuards, subscriber: Option<Uuid>, topic: &str) -> Result<(), Rejection> {
    match guards.may_subscribe(subscriber, topic).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(Rejection::new(Code::PermissionDenied, "TOPIC_FORBIDDEN")),
        // The guard could not decide (its database is down, say): refuse
        // for now rather than let someone in unchecked.
        Err(error) => {
            tracing::warn!(topic, %error, "topic guard failed");
            Err(Rejection::new(Code::Unavailable, "TOPIC_CHECK_FAILED"))
        }
    }
}

/// Checks the shape of a topic name: a lowercase kind, then one or more
/// non-empty segments, using only characters that map one-to-one onto bus
/// subjects (letters, digits, `-`, `_`, with `:` between segments).
pub(crate) fn validate(topic: &str) -> Result<(), Rejection> {
    if topic.len() > MAX_TOPIC_LEN {
        return Err(INVALID);
    }
    let mut segments = topic.split(':');
    let kind = segments.next().unwrap_or_default();
    let kind_ok = kind.starts_with(|c: char| c.is_ascii_lowercase()) && kind.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    let mut ids = 0;
    for segment in segments {
        if segment.is_empty() || !segment.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(INVALID);
        }
        ids += 1;
    }
    if kind_ok && ids > 0 { Ok(()) } else { Err(INVALID) }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokuly_core::{
        AppError, AppResult,
        realtime::{TopicGuard, topic_subject},
    };

    use super::*;

    #[test]
    fn documented_topics_are_valid() {
        for topic in [
            "live:0190d4f6-6f43-7cc2-9d6b-6f6f6f6f6f6f",
            "party:0190d4f6-6f43-7cc2-9d6b-6f6f6f6f6f6f",
            "subject:video:0190d4f6-6f43-7cc2-9d6b-6f6f6f6f6f6f",
            "share:Ab3_x-9Z",
            "asset:0190d4f66f437cc29d6b6f6f6f6f6f6f",
        ] {
            assert!(validate(topic).is_ok(), "{topic}");
            // Valid names reach the bus unchanged, so two topics never share a subject.
            assert_eq!(topic_subject(topic), format!("rt.t.{topic}"));
        }
    }

    #[test]
    fn malformed_topics_are_rejected() {
        let too_long = format!("live:{}", "a".repeat(MAX_TOPIC_LEN));
        for topic in ["", "live", "live:", ":abc", "live::abc", "Live:abc", "9live:abc", "live:a b", "live:a.b", "live:*", "live:>", "live:ab\u{e9}", too_long.as_str()] {
            assert_eq!(validate(topic), Err(INVALID), "{topic}");
        }
    }

    struct MembersOnly(Uuid);

    #[async_trait]
    impl TopicGuard for MembersOnly {
        async fn may_subscribe(&self, subscriber: Option<Uuid>, _topic: &str) -> AppResult<bool> {
            Ok(subscriber == Some(self.0))
        }
    }

    struct Broken;

    #[async_trait]
    impl TopicGuard for Broken {
        async fn may_subscribe(&self, _subscriber: Option<Uuid>, _topic: &str) -> AppResult<bool> {
            Err(AppError::unavailable("DB_DOWN", "cannot check"))
        }
    }

    #[tokio::test]
    async fn guard_verdicts_become_rejections() {
        let member = Uuid::now_v7();
        let guards = TopicGuards::default();
        guards.register("party", Arc::new(MembersOnly(member)));
        guards.register("vault", Arc::new(Broken));

        assert!(authorize(&guards, Some(member), "party:1").await.is_ok());
        for outsider in [None, Some(Uuid::now_v7())] {
            let denied = authorize(&guards, outsider, "party:1").await.unwrap_err();
            assert_eq!((denied.code, denied.reason), (Code::PermissionDenied, "TOPIC_FORBIDDEN"));
        }
        // A guard that cannot decide lets nobody in.
        let undecided = authorize(&guards, Some(member), "vault:1").await.unwrap_err();
        assert_eq!((undecided.code, undecided.reason), (Code::Unavailable, "TOPIC_CHECK_FAILED"));
        // No guard for `live`: public, also for anonymous connections.
        assert!(authorize(&guards, None, "live:1").await.is_ok());
    }
}
