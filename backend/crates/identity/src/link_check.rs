//! Verified profile links: a link is marked verified while the page it
//! points to links back to the profile with `rel="me"` (the convention
//! Mastodon and Misskey use), so a visitor can tell that the YouTube
//! channel or blog on a profile is really that person's.
//!
//! The check runs in the background (job [`crate::extras::LINK_CHECK_JOB`]),
//! after links change, when the handle changes (the profile URL moves with
//! it), on request (`RecheckProfileLinks`) and again for every link a week
//! after its last check. At most 20 new links per account and day are
//! checked (more wait for the next day) and at most two pages are fetched
//! from one host at once. Pages are fetched through media's guarded fetcher
//! (`tokuly_core::egress` underneath): public addresses only, every redirect
//! hop checked, at most 512 KiB read, five seconds per page.
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;
use tokuly_media::link_preview::fetch;
use url::Url;

use crate::Identity;

/// What a page may weigh before the rest is ignored.
const MAX_PAGE_BYTES: usize = 512 * 1024;
/// New links checked per account and day; more wait for the next day.
pub const NEW_LINK_CHECKS_PER_DAY: u32 = 20;
/// Pages fetched from one host at once, per process.
pub const PER_HOST_FETCHES: usize = 2;
/// A checked link is checked again after this long.
const RECHECK_AFTER_DAYS: i32 = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkCheckJob {
    pub user_id: Uuid,
}

/// The `href`s of the `<a>` and `<link>` elements of `html` whose `rel`
/// contains `me`, resolved against `base`.
pub fn rel_me_targets(html: &str, base: &Url) -> Vec<Url> {
    static TAG: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#"(?is)<(?:a|link)\b([^>]*)>"#).expect("static regex"));
    static ATTR: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(?is)([a-z][a-z0-9-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))"#).expect("static regex")
    });
    let mut targets = Vec::new();
    for tag in TAG.captures_iter(html) {
        let (mut rel, mut href) = (None, None);
        for attr in ATTR.captures_iter(&tag[1]) {
            let value = attr.get(2).or(attr.get(3)).or(attr.get(4)).map(|m| m.as_str()).unwrap_or_default();
            match attr[1].to_ascii_lowercase().as_str() {
                "rel" => rel = Some(value.to_ascii_lowercase()),
                "href" => href = Some(value.trim().replace("&amp;", "&")),
                _ => {}
            }
        }
        if rel.is_some_and(|rel| rel.split_whitespace().any(|token| token == "me"))
            && let Some(href) = href
            && let Ok(url) = base.join(&href)
        {
            targets.push(url);
        }
    }
    targets
}

/// Whether `target` is the profile of `handle` on this server
/// (`<public url>/u/<handle>` or `/@<handle>`, any case, with or without a
/// trailing slash, query or fragment).
pub fn is_profile_url(target: &Url, public_url: &str, handle: &str) -> bool {
    let Ok(ours) = Url::parse(public_url) else { return false };
    let same_origin = target.scheme() == ours.scheme()
        && target.host_str().map(str::to_ascii_lowercase) == ours.host_str().map(str::to_ascii_lowercase)
        && target.port_or_known_default() == ours.port_or_known_default();
    if !same_origin {
        return false;
    }
    let base = ours.path().trim_end_matches('/');
    let path = target.path().trim_end_matches('/');
    let Some(rest) = path.strip_prefix(base) else { return false };
    let rest = rest.to_ascii_lowercase();
    let handle = handle.to_ascii_lowercase();
    rest == format!("/u/{handle}") || rest == format!("/@{handle}")
}

impl Identity {
    /// Checks the links of one account that are due: never checked, or last
    /// checked a week ago. Each answer is stored at once; a page that
    /// cannot be fetched counts as no link back (and is tried again next
    /// week). Repeating the job is harmless.
    pub async fn check_profile_links(&self, user_id: Uuid) -> AppResult<usize> {
        let Some(user) = crate::users::find_by_id(&self.core, user_id).await?.filter(|u| u.is_active()) else {
            return Ok(0);
        };
        let due: Vec<(Uuid, String, bool)> = sqlx::query_as(
            "SELECT id, url, checked_at IS NULL FROM profile_links
             WHERE user_id = $1 AND (checked_at IS NULL OR checked_at < now() - make_interval(days => $2))
             ORDER BY position",
        )
        .bind(user_id)
        .bind(RECHECK_AFTER_DAYS)
        .fetch_all(&self.core.db)
        .await?;
        let policy = self.link_policy();
        let client =
            fetch::guarded_client(policy, "TokulyBot/1.0 (+profile link check)").map_err(AppError::internal)?;
        let mut checked = 0;
        for (link_id, url, new) in &due {
            // Saving links again and again must not make the server fetch
            // pages without end: new links get a daily budget per account;
            // the rest wait for the sweep (the next day).
            if *new
                && self
                    .core
                    .cache
                    .rate_limit(
                        &format!("link-check-new:{user_id}"),
                        NEW_LINK_CHECKS_PER_DAY,
                        Duration::from_secs(24 * 3600),
                    )
                    .await
                    .is_err()
            {
                tracing::debug!(%user_id, "daily budget of new link checks used up");
                break;
            }
            let verified = match Url::parse(url) {
                Ok(url) => {
                    // Few requests at a time to any one site, whoever's links point there.
                    let _slot = self.link_host_slot(url.host_str().unwrap_or_default()).await;
                    match fetch::fetch_limited(&client, policy, url, "text/html,application/xhtml+xml", MAX_PAGE_BYTES)
                        .await
                    {
                        Ok(page) => {
                            let html = String::from_utf8_lossy(&page.body);
                            rel_me_targets(&html, &page.url)
                                .iter()
                                .any(|target| is_profile_url(target, self.core.cfg.public_url(), &user.handle))
                        }
                        Err(e) => {
                            tracing::debug!(%user_id, %link_id, error = %e, "profile link not fetched");
                            false
                        }
                    }
                }
                Err(_) => false,
            };
            // Only the link as it still is: an edit in between starts over.
            sqlx::query(
                "UPDATE profile_links SET checked_at = now(),
                     verified_at = CASE WHEN $3 THEN COALESCE(verified_at, now()) END
                 WHERE id = $1 AND url = $2",
            )
            .bind(link_id)
            .bind(url)
            .bind(verified)
            .execute(&self.core.db)
            .await?;
            checked += 1;
        }
        Ok(checked)
    }

    /// The handle changed: the profile URL moved, so every link must link
    /// back to the new one.
    pub(crate) async fn links_moved(&self, user_id: Uuid) -> AppResult<()> {
        sqlx::query("UPDATE profile_links SET checked_at = NULL WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        self.enqueue_link_check(user_id).await;
        Ok(())
    }

    /// Queues the check of accounts whose links are due (the weekly
    /// recheck, and links whose first check was never queued).
    pub async fn sweep_profile_links(&self) -> AppResult<usize> {
        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT user_id FROM profile_links
             WHERE checked_at IS NULL OR checked_at < now() - make_interval(days => $1)
             LIMIT 500",
        )
        .bind(RECHECK_AFTER_DAYS)
        .fetch_all(&self.core.db)
        .await?;
        for user_id in &due {
            self.enqueue_link_check(*user_id).await;
        }
        Ok(due.len())
    }

    /// A slot for fetching from `host`: at most [`PER_HOST_FETCHES`] at once
    /// in this process (each worker process keeps its own count).
    async fn link_host_slot(&self, host: &str) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let semaphore = {
            let mut hosts = self.link_hosts.lock();
            // Idle hosts are dropped now and then, so the map stays small.
            if hosts.len() > 1024 {
                hosts.retain(|_, s| s.available_permits() < PER_HOST_FETCHES);
            }
            hosts
                .entry(host.to_ascii_lowercase())
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Semaphore::new(PER_HOST_FETCHES)))
                .clone()
        };
        semaphore.acquire_owned().await.ok()
    }

    fn link_policy(&self) -> tokuly_core::egress::Policy {
        tokuly_core::egress::Policy { allow_loopback: self.cfg.profile_links_allow_loopback }
    }
}

/// How often the sweep runs.
pub(crate) const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 3600);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_rel_me_links_in_anchors_and_link_elements() {
        let base = Url::parse("https://blog.example.com/about/").unwrap();
        let html = r#"<html><head><LINK REL="me authn" href="https://tokuly.app/u/Yui"></head>
            <body><a href='/other' rel=nofollow>x</a><a rel="noopener me" href="https://tokuly.app/@yui?ref=1">me</a>
            <a href="https://tokuly.app/u/someone">not me</a></body></html>"#;
        let targets: Vec<String> = rel_me_targets(html, &base).into_iter().map(|u| u.to_string()).collect();
        assert_eq!(targets, ["https://tokuly.app/u/Yui", "https://tokuly.app/@yui?ref=1"]);
    }

    #[test]
    fn only_this_servers_profile_of_this_handle_counts() {
        let public = "https://tokuly.app";
        let ok = |u: &str| is_profile_url(&Url::parse(u).unwrap(), public, "yui");
        assert!(ok("https://tokuly.app/u/yui"));
        assert!(ok("https://TOKULY.app/u/YUI/"));
        assert!(ok("https://tokuly.app/@yui#top"));
        assert!(!ok("https://tokuly.app/u/yuiko"));
        assert!(!ok("http://tokuly.app/u/yui"));
        assert!(!ok("https://evil.example/u/yui"));
        assert!(!ok("https://tokuly.app.evil.example/u/yui"));
        let prefixed = Url::parse("https://example.com/app/u/yui").unwrap();
        assert!(is_profile_url(&prefixed, "https://example.com/app/", "yui"));
    }
}
