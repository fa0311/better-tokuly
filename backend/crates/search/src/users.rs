//! User documents.
//!
//! Identity does not publish users as search documents, so this crate builds
//! them from identity's `users` table (read-only): whenever identity says an
//! account changed ([`Search::index_user`]) and in full on request
//! ([`Search::reindex_users`], job `search.reindex.users`).
//!
//! An account is searchable when it is active (not suspended, not deleted)
//! and its privacy settings say `discoverable`. Besides the name and bio,
//! the public parts of its プロフ帳 are found: the long bio as text, its
//! favourite things and 推し as tags (`tokuly_identity::extras::search_terms`).
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokuly_core::{outbound::SearchDoc, prelude::*};
use tokuly_identity::{extras::ProfileSearchTerms, settings::privacy_from_json};

use crate::{Search, document::IndexDoc, kinds};

/// Job that rebuilds every user document.
pub const REINDEX_USERS_JOB: &str = "search.reindex.users";

/// Users read per step of a full reindex.
const REINDEX_CHUNK: i64 = 500;
/// What a verified badge is worth next to profile views when ranking
/// accounts of the same name: the real one should come first.
const VERIFIED_BONUS: f64 = 1000.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReindexUsersJob {}

/// What a full user reindex did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReindexUsersReport {
    /// Accounts looked at.
    pub scanned: u64,
    /// Documents stored because they were new or had changed.
    pub stored: u64,
    /// Documents removed because the account is no longer searchable.
    pub removed: u64,
}

#[derive(sqlx::FromRow)]
struct UserDocRow {
    id: Uuid,
    handle: String,
    display_name: String,
    bio: String,
    avatar: Option<serde_json::Value>,
    verified: bool,
    privacy: serde_json::Value,
    profile_view_count: i64,
    banned_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

macro_rules! user_doc_columns {
    () => {
        "id, handle::text AS handle, display_name, bio, avatar, verified, privacy, profile_view_count, \
         banned_at, deleted_at, created_at"
    };
}

impl UserDocRow {
    fn searchable(&self) -> bool {
        self.deleted_at.is_none() && self.banned_at.is_none() && privacy_from_json(&self.privacy).discoverable
    }

    /// The account's search document; `None` when it must not be found.
    fn document(&self, terms: Option<&ProfileSearchTerms>) -> Option<IndexDoc> {
        if !self.searchable() {
            return None;
        }
        // `avatar` is a `tokuly.common.v1.Image` as proto JSON.
        let avatar_url = self.avatar.as_ref().and_then(|a| a.get("url")).and_then(|u| u.as_str()).map(str::to_owned);
        let title = if self.display_name.trim().is_empty() { self.handle.clone() } else { self.display_name.clone() };
        Some(IndexDoc::from_search_doc(SearchDoc {
            kind: kinds::USER.to_owned(),
            id: self.id,
            title,
            text: match terms.map(|t| t.text.as_str()).filter(|t| !t.is_empty()) {
                Some(long_bio) => format!("{} {long_bio}", self.bio),
                None => self.bio.clone(),
            },
            tags: terms.map(|t| t.tags.clone()).unwrap_or_default(),
            owner_id: Some(self.id),
            public: true,
            image_url: avatar_url,
            path: format!("/u/{}", self.handle),
            created_at: self.created_at.timestamp(),
            score: self.profile_view_count.max(0) as f64 + if self.verified { VERIFIED_BONUS } else { 0.0 },
            extra: serde_json::json!({ "handle": self.handle, "verified": self.verified }),
        }))
    }
}

impl Search {
    /// Brings one user's document in line with the account as it is now:
    /// stored when searchable, removed otherwise (also for unknown users).
    pub async fn index_user(&self, version: Uuid, user_id: Uuid) -> AppResult<()> {
        let row: Option<UserDocRow> =
            sqlx::query_as(concat!("SELECT ", user_doc_columns!(), " FROM users WHERE id = $1"))
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        let terms = tokuly_identity::extras::search_terms(&self.core, &[user_id]).await?;
        match row.as_ref().and_then(|row| row.document(terms.get(&user_id))) {
            Some(doc) => self.store_document(version, doc).await?,
            None => self.remove_document(version, kinds::USER, user_id).await?,
        };
        Ok(())
    }

    /// Rebuilds every user document from identity's data (the backfill and
    /// the `user` half of an administrator's reindex). Accounts are read in
    /// ID order; documents that did not change are not written to the engine
    /// again.
    pub async fn reindex_users(&self) -> AppResult<ReindexUsersReport> {
        let mut report = ReindexUsersReport::default();
        let mut after: Option<Uuid> = None;
        loop {
            // Taken before the read: an account changed after this instant
            // emits an event with a newer ID, which then wins.
            let version = Uuid::now_v7();
            let rows: Vec<UserDocRow> = sqlx::query_as(concat!(
                "SELECT ",
                user_doc_columns!(),
                " FROM users WHERE ($1::uuid IS NULL OR id > $1) ORDER BY id LIMIT $2"
            ))
            .bind(after)
            .bind(REINDEX_CHUNK)
            .fetch_all(&self.core.db)
            .await?;
            let Some(last) = rows.last() else { break };
            after = Some(last.id);
            report.scanned += rows.len() as u64;
            let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
            let terms = tokuly_identity::extras::search_terms(&self.core, &ids).await?;
            let mut docs = Vec::new();
            let mut gone = Vec::new();
            for row in &rows {
                match row.document(terms.get(&row.id)) {
                    Some(doc) => docs.push(doc),
                    None => gone.push(row.id),
                }
            }
            report.stored += self.store_documents(version, kinds::USER, &docs).await?;
            report.removed += self.remove_existing(version, kinds::USER, &gone).await?;
            if (rows.len() as i64) < REINDEX_CHUNK {
                break;
            }
        }
        // Documents of accounts that no longer exist at all.
        let orphaned = sqlx::query(
            "UPDATE search_documents d
             SET version = $1, owner_id = NULL, doc = NULL, generation = generation + 1, queued_at = now(), updated_at = now()
             WHERE d.kind = 'user' AND d.doc IS NOT NULL AND d.version < $1
               AND NOT EXISTS (SELECT 1 FROM users u WHERE u.id = d.doc_id)",
        )
        .bind(Uuid::now_v7())
        .execute(&self.core.db)
        .await?;
        report.removed += orphaned.rows_affected();
        self.indexer_wake.notify_one();
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> UserDocRow {
        UserDocRow {
            id: Uuid::now_v7(),
            handle: "toku_zou".into(),
            display_name: "とくぞう".into(),
            bio: "渋谷によくいます".into(),
            avatar: Some(serde_json::json!({ "assetId": "a", "url": "https://cdn.example/a.webp", "width": 256 })),
            verified: false,
            privacy: serde_json::json!({}),
            profile_view_count: 40,
            banned_at: None,
            deleted_at: None,
            created_at: DateTime::from_timestamp(1_750_000_000, 0).unwrap(),
        }
    }

    #[test]
    fn an_active_discoverable_account_becomes_a_document() {
        let user = row();
        let doc = user.document(None).expect("searchable");
        assert_eq!(doc.kind, "user");
        assert_eq!(doc.id, user.id);
        assert_eq!(doc.owner_id, Some(user.id), "blocking the user hides their own document");
        assert_eq!(doc.title, "とくぞう");
        assert_eq!(doc.handle.as_deref(), Some("toku_zou"));
        assert_eq!(doc.text, "渋谷によくいます");
        assert_eq!(doc.path, "/u/toku_zou");
        assert_eq!(doc.image_url.as_deref(), Some("https://cdn.example/a.webp"));
        assert_eq!(doc.created_at, 1_750_000_000);
        assert_eq!(doc.score, 40.0);
        assert_eq!(doc.extra, serde_json::json!({ "handle": "toku_zou", "verified": false }));
    }

    #[test]
    fn public_profile_extras_are_found_too() {
        let terms = ProfileSearchTerms {
            text: "ヨルシカが好き".into(),
            tags: vec!["ヨルシカ".into(), "ラーメン".into()],
        };
        let doc = row().document(Some(&terms)).unwrap();
        assert_eq!(doc.text, "渋谷によくいます ヨルシカが好き");
        assert_eq!(doc.tags, ["ヨルシカ", "ラーメン"]);
        assert_eq!(row().document(Some(&ProfileSearchTerms::default())).unwrap().text, "渋谷によくいます");
    }

    #[test]
    fn suspended_deleted_and_hidden_accounts_are_not_searchable() {
        let banned = UserDocRow { banned_at: Some(Utc::now()), ..row() };
        assert!(banned.document(None).is_none());
        let deleted = UserDocRow { deleted_at: Some(Utc::now()), ..row() };
        assert!(deleted.document(None).is_none());
        let hidden =
            UserDocRow { privacy: serde_json::json!({ "discoverable": false, "shareBattery": true }), ..row() };
        assert!(hidden.document(None).is_none());
        let explicit = UserDocRow { privacy: serde_json::json!({ "discoverable": true }), ..row() };
        assert!(explicit.document(None).is_some());
    }

    #[test]
    fn a_verified_account_outranks_its_namesakes_and_a_nameless_one_uses_its_handle() {
        let verified = UserDocRow { verified: true, ..row() };
        assert_eq!(verified.document(None).unwrap().score, 40.0 + VERIFIED_BONUS);
        let nameless = UserDocRow { display_name: "  ".into(), avatar: None, ..row() };
        let doc = nameless.document(None).unwrap();
        assert_eq!(doc.title, "toku_zou");
        assert_eq!(doc.image_url, None);
    }
}
