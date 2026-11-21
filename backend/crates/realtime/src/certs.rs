//! The TLS identity of the WebTransport listener.
//!
//! Two sources:
//!
//! - **PEM files** (`TOKULY_RT_TLS_CERT` / `TOKULY_RT_TLS_KEY`): a publicly
//!   trusted certificate managed by the operator. The files are re-read
//!   periodically so renewals are picked up without a restart.
//! - **Self-signed** (the default): an ECDSA P-256 certificate that browsers
//!   accept only when its SHA-256 hash is passed as
//!   `serverCertificateHashes`, and only if it is valid for less than 14
//!   days. The gateway therefore replaces it on a schedule and publishes the
//!   hashes in Valkey, where every API instance's `GetEndpoints` finds them.
//!   The next certificate's hash is published ahead of the switch, so a
//!   client holding a slightly old `GetEndpoints` answer still connects.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tokuly_core::{AppResult, Core, cache::Cache};
use wtransport::{
    Identity,
    tls::self_signed::time::{Duration as CertDuration, OffsetDateTime},
};

use crate::{keys, wt::WtListener};

/// How often published hashes are renewed and the schedule is checked
/// (more often for very short-lived certificates, see [`check_interval`]).
const PUBLISH_INTERVAL: Duration = Duration::from_secs(30);
/// How long a published hash outlives its gateway.
const PUBLISH_TTL: Duration = Duration::from_secs(90);
/// How often PEM files are checked for a renewed certificate.
const FILE_CHECK_INTERVAL: Duration = Duration::from_secs(300);
/// Certificates start slightly in the past so clients with a slow clock accept them.
const BACKDATE: Duration = Duration::from_secs(300);
/// Fraction of a certificate's lifetime after which the next one is announced...
const ANNOUNCE_AT: f64 = 0.60;
/// ...and after which the listener switches to it...
const REPLACE_AT: f64 = 0.75;
/// ...provided the next one has been announced for this long, so clients
/// that just fetched hashes are not caught out...
const MIN_LEAD: f64 = 0.05;
/// ...unless the current certificate is this close to expiring.
const LAST_CALL: f64 = 0.85;

type Hash = [u8; 32];

/// What the rotation schedule asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Due {
    Nothing,
    /// Generate the next certificate and publish its hash.
    Announce,
    /// Switch the listener to the next certificate.
    Replace,
}

/// The schedule for a certificate of the given age, valid for `validity`,
/// whose successor has been announced for `announced_for` (if at all).
fn due(age: Duration, validity: Duration, announced_for: Option<Duration>) -> Due {
    let at = |fraction: f64| validity.mul_f64(fraction);
    match announced_for {
        None if age >= at(ANNOUNCE_AT) => Due::Announce,
        Some(announced_for) if age >= at(REPLACE_AT) && (announced_for >= at(MIN_LEAD) || age >= at(LAST_CALL)) => Due::Replace,
        _ => Due::Nothing,
    }
}

/// How often the schedule is checked: often enough that no step of it can
/// be late by a meaningful part of the certificate's lifetime.
fn check_interval(validity: Duration) -> Duration {
    PUBLISH_INTERVAL.min(validity / 40)
}

struct SelfSigned {
    identity: Identity,
    hash: Hash,
    issued: Instant,
    not_after: DateTime<Utc>,
}

enum Source {
    Files { cert: PathBuf, key: PathBuf, leaf: Option<Hash>, checked: Instant },
    SelfSigned { current: SelfSigned, upcoming: Option<SelfSigned> },
}

pub(crate) struct Certificates {
    core: Core,
    files: Option<(PathBuf, PathBuf)>,
    validity: Duration,
    /// Subject alternative names of self-signed certificates.
    names: Vec<String>,
    /// `None` until the listener asked for its first identity.
    source: Mutex<Option<Source>>,
}

impl Certificates {
    pub(crate) fn new(core: Core, files: Option<(PathBuf, PathBuf)>, validity: Duration, names: Vec<String>) -> Self {
        Self { core, files, validity, names, source: Mutex::new(None) }
    }

    /// The identity the listener starts with.
    pub(crate) async fn load(&self) -> anyhow::Result<Identity> {
        let (source, identity) = match &self.files {
            Some((cert, key)) => {
                let identity = load_files(cert, key).await?;
                let source = Source::Files { cert: cert.clone(), key: key.clone(), leaf: leaf_hash(&identity), checked: Instant::now() };
                (source, identity)
            }
            None => {
                let current = self.issue()?;
                let identity = current.identity.clone_identity();
                (Source::SelfSigned { current, upcoming: None }, identity)
            }
        };
        *self.source.lock().await = Some(source);
        Ok(identity)
    }

    fn issue(&self) -> anyhow::Result<SelfSigned> {
        let backdate = BACKDATE.min(self.validity / 10);
        let not_before = OffsetDateTime::now_utc() - CertDuration::try_from(backdate)?;
        let not_after = not_before + CertDuration::try_from(self.validity)?;
        let build = |names: &[String]| Identity::self_signed_builder().subject_alt_names(names).not_before(not_before).not_after(not_after).build();
        // With pinned hashes the names are not what clients verify, so a
        // public host name that cannot be put in a certificate is not fatal.
        let identity = build(&self.names)
            .or_else(|_| build(&local_names()))
            .map_err(|e| anyhow::anyhow!("cannot generate a self-signed certificate: {e}"))?;
        let hash = leaf_hash(&identity).ok_or_else(|| anyhow::anyhow!("generated identity has no certificate"))?;
        let not_after = DateTime::from_timestamp(not_after.unix_timestamp(), 0).unwrap_or_default();
        Ok(SelfSigned { identity, hash, issued: Instant::now(), not_after })
    }

    /// Generates the next self-signed certificate (if there is none yet) and
    /// publishes its hash next to the current one. No-op with PEM files.
    pub(crate) async fn announce_next(&self) -> anyhow::Result<()> {
        {
            let mut source = self.source.lock().await;
            if let Some(Source::SelfSigned { upcoming, .. }) = source.as_mut()
                && upcoming.is_none()
            {
                *upcoming = Some(self.issue()?);
                tracing::info!("next WebTransport certificate announced");
            }
        }
        self.publish().await?;
        Ok(())
    }

    /// Switches the listener to the announced certificate and stops
    /// publishing the old hash. Connections already open are unaffected.
    /// No-op when nothing was announced.
    pub(crate) async fn activate_next(&self, listener: &WtListener) -> anyhow::Result<()> {
        let retired = {
            let mut source = self.source.lock().await;
            let Some(Source::SelfSigned { current, upcoming }) = source.as_mut() else { return Ok(()) };
            let Some(next) = upcoming.take() else { return Ok(()) };
            if let Err(error) = listener.use_identity(next.identity.clone_identity()) {
                *upcoming = Some(next);
                return Err(error);
            }
            std::mem::replace(current, next).hash
        };
        tracing::info!("WebTransport certificate rotated");
        let mut conn = self.core.cache.conn();
        redis::cmd("ZREM").arg(keys::CERT_HASHES).arg(hex::encode(retired)).query_async::<()>(&mut conn).await?;
        self.publish().await?;
        Ok(())
    }

    /// Writes this gateway's hashes (current and announced) with a short
    /// expiry; called again before they run out.
    pub(crate) async fn publish(&self) -> anyhow::Result<()> {
        let entries: Vec<(Hash, DateTime<Utc>)> = match self.source.lock().await.as_ref() {
            Some(Source::SelfSigned { current, upcoming }) => {
                std::iter::once(current).chain(upcoming).map(|cert| (cert.hash, cert.not_after)).collect()
            }
            _ => return Ok(()),
        };
        let now = Utc::now().timestamp_millis();
        let mut pipe = redis::pipe();
        pipe.cmd("ZREMRANGEBYSCORE").arg(keys::CERT_HASHES).arg("-inf").arg(now).ignore();
        for (hash, not_after) in entries {
            // Never advertise a certificate beyond its own expiry.
            let until = (now + PUBLISH_TTL.as_millis() as i64).min(not_after.timestamp_millis());
            pipe.cmd("ZADD").arg(keys::CERT_HASHES).arg(until).arg(hex::encode(hash)).ignore();
        }
        pipe.cmd("PEXPIRE").arg(keys::CERT_HASHES).arg(PUBLISH_TTL.as_millis() as i64).ignore();
        pipe.query_async::<()>(&mut self.core.cache.conn()).await?;
        Ok(())
    }

    /// Hashes clients should currently pin: every gateway's current
    /// certificate plus those about to be switched to.
    pub(crate) async fn published(cache: &Cache) -> AppResult<Vec<Vec<u8>>> {
        let now = Utc::now().timestamp_millis();
        let hashes: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(keys::CERT_HASHES)
            .arg(format!("({now}"))
            .arg("+inf")
            .query_async(&mut cache.conn())
            .await?;
        Ok(hashes.iter().filter_map(|hash| hex::decode(hash).ok()).filter(|hash| hash.len() == 32).collect())
    }

    /// Keeps the hashes published and the certificate fresh until `stop`.
    pub(crate) async fn run(&self, listener: &WtListener, stop: CancellationToken) {
        let mut tick = tokio::time::interval(check_interval(self.validity));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = tick.tick() => {}
            }
            if let Err(error) = self.maintain(listener).await {
                tracing::error!(error = %format!("{error:#}"), "WebTransport certificate maintenance failed");
            }
        }
        self.withdraw().await;
    }

    async fn maintain(&self, listener: &WtListener) -> anyhow::Result<()> {
        enum Step {
            Rotation(Due),
            ReloadFiles(PathBuf, PathBuf),
            Nothing,
        }
        let step = match self.source.lock().await.as_mut() {
            Some(Source::SelfSigned { current, upcoming }) => {
                let announced_for = upcoming.as_ref().map(|next| next.issued.elapsed());
                Step::Rotation(due(current.issued.elapsed(), self.validity, announced_for))
            }
            Some(Source::Files { cert, key, checked, .. }) if checked.elapsed() >= FILE_CHECK_INTERVAL => {
                *checked = Instant::now();
                Step::ReloadFiles(cert.clone(), key.clone())
            }
            _ => Step::Nothing,
        };
        match step {
            // Nothing due: just keep the published hashes from expiring.
            Step::Rotation(Due::Nothing) => self.publish().await,
            Step::Rotation(Due::Announce) => self.announce_next().await,
            Step::Rotation(Due::Replace) => self.activate_next(listener).await,
            Step::ReloadFiles(cert, key) => self.reload_files(listener, &cert, &key).await,
            Step::Nothing => Ok(()),
        }
    }

    /// Picks up a certificate that was renewed on disk.
    async fn reload_files(&self, listener: &WtListener, cert: &PathBuf, key: &PathBuf) -> anyhow::Result<()> {
        let identity = load_files(cert, key).await?;
        let renewed = leaf_hash(&identity);
        let mut source = self.source.lock().await;
        if let Some(Source::Files { leaf, .. }) = source.as_mut()
            && *leaf != renewed
        {
            listener.use_identity(identity)?;
            *leaf = renewed;
            tracing::info!(cert = %cert.display(), "WebTransport certificate reloaded from disk");
        }
        Ok(())
    }

    /// Shutdown: clients should no longer be told about this gateway's certificates.
    async fn withdraw(&self) {
        let hashes: Vec<String> = match self.source.lock().await.as_ref() {
            Some(Source::SelfSigned { current, upcoming }) => {
                std::iter::once(current).chain(upcoming).map(|cert| hex::encode(cert.hash)).collect()
            }
            _ => return,
        };
        let mut conn = self.core.cache.conn();
        if let Err(error) = redis::cmd("ZREM").arg(keys::CERT_HASHES).arg(hashes).query_async::<()>(&mut conn).await {
            tracing::warn!(%error, "certificate hashes could not be withdrawn");
        }
    }
}

async fn load_files(cert: &PathBuf, key: &PathBuf) -> anyhow::Result<Identity> {
    let identity = Identity::load_pemfiles(cert, key)
        .await
        .map_err(|e| anyhow::anyhow!("cannot load the TLS identity from {} and {}: {e}", cert.display(), key.display()))?;
    anyhow::ensure!(leaf_hash(&identity).is_some(), "{} contains no certificate", cert.display());
    Ok(identity)
}

/// SHA-256 of the DER encoding of the certificate presented to clients.
fn leaf_hash(identity: &Identity) -> Option<Hash> {
    identity.certificate_chain().as_slice().first().map(|certificate| *certificate.hash().as_ref())
}

/// Names every self-signed certificate carries besides the public host.
pub(crate) fn local_names() -> Vec<String> {
    ["localhost", "127.0.0.1", "::1"].map(str::to_owned).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);
    const DAY: Duration = Duration::from_secs(24 * 3600);

    #[test]
    fn the_next_certificate_is_announced_well_before_it_is_used() {
        let validity = 13 * DAY;
        assert_eq!(due(Duration::ZERO, validity, None), Due::Nothing);
        assert_eq!(due(7 * DAY, validity, None), Due::Nothing);
        // 60% of 13 days = 7.8 days: announce, once.
        assert_eq!(due(8 * DAY, validity, None), Due::Announce);
        assert_eq!(due(8 * DAY, validity, Some(HOUR)), Due::Nothing);
        // 75% = 9.75 days: switch to the certificate announced two days ago.
        assert_eq!(due(9 * DAY, validity, Some(DAY)), Due::Nothing);
        assert_eq!(due(10 * DAY, validity, Some(2 * DAY)), Due::Replace);
    }

    #[test]
    fn a_late_announcement_still_gets_its_lead_time() {
        let validity = 13 * DAY;
        // The switch is overdue but nothing was announced: announce first...
        assert_eq!(due(10 * DAY, validity, None), Due::Announce);
        // ...wait out the lead (5% = 15.6 hours)...
        assert_eq!(due(10 * DAY + HOUR, validity, Some(HOUR)), Due::Nothing);
        assert_eq!(due(10 * DAY + 16 * HOUR, validity, Some(16 * HOUR)), Due::Replace);
        // ...unless the current certificate is about to expire (85% = 11.05 days).
        assert_eq!(due(12 * DAY, validity, Some(HOUR)), Due::Replace);
    }

    #[test]
    fn a_certificate_is_always_replaced_before_it_expires() {
        for validity in [Duration::from_secs(6), HOUR, DAY, 13 * DAY] {
            // The latest possible switch: last call, noticed one check late.
            let latest_switch = validity.mul_f64(LAST_CALL) + check_interval(validity);
            let expires = validity - BACKDATE.min(validity / 10);
            assert!(latest_switch < expires, "{validity:?}");
        }
    }
}
