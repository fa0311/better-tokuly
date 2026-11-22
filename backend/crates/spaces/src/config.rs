//! Settings of the spaces domain.
//!
//! The server flattens every domain's settings into one command line, so
//! each argument carries a `spaces` prefix in its flag and in clap's
//! argument ID. LiveKit itself is configured once for the whole server
//! (`TOKULY_LIVEKIT_*`, see `tokuly_core::livekit`).
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct SpacesConfig {
    /// Seats on a room's stage, the host's included.
    #[arg(
        id = "spaces_max_speakers",
        long = "spaces-max-speakers",
        env = "TOKULY_SPACES_MAX_SPEAKERS",
        default_value_t = 10
    )]
    pub max_speakers: i32,

    /// Co-hosts a room can have.
    #[arg(
        id = "spaces_max_cohosts",
        long = "spaces-max-cohosts",
        env = "TOKULY_SPACES_MAX_COHOSTS",
        default_value_t = 2
    )]
    pub max_cohosts: i32,

    /// People a room holds, the stage included.
    #[arg(
        id = "spaces_max_listeners",
        long = "spaces-max-listeners",
        env = "TOKULY_SPACES_MAX_LISTENERS",
        default_value_t = 3000
    )]
    pub max_listeners: i32,

    /// Seconds a live room waits for a host who is not inside before it is
    /// handed to a co-host or ended.
    #[arg(
        id = "spaces_host_grace_seconds",
        long = "spaces-host-grace-seconds",
        env = "TOKULY_SPACES_HOST_GRACE_SECONDS",
        default_value_t = 180
    )]
    pub host_grace_seconds: i64,

    /// Seconds a live room may stay empty before it is ended.
    #[arg(
        id = "spaces_empty_grace_seconds",
        long = "spaces-empty-grace-seconds",
        env = "TOKULY_SPACES_EMPTY_GRACE_SECONDS",
        default_value_t = 120
    )]
    pub empty_grace_seconds: i64,

    /// Seconds someone who entered may take to connect to LiveKit before
    /// they count as gone.
    #[arg(
        id = "spaces_connect_grace_seconds",
        long = "spaces-connect-grace-seconds",
        env = "TOKULY_SPACES_CONNECT_GRACE_SECONDS",
        default_value_t = 45
    )]
    pub connect_grace_seconds: i64,

    /// Seconds someone who was connected may be missing from LiveKit (a
    /// reconnect, a change of network) before they count as gone.
    #[arg(
        id = "spaces_reconnect_grace_seconds",
        long = "spaces-reconnect-grace-seconds",
        env = "TOKULY_SPACES_RECONNECT_GRACE_SECONDS",
        default_value_t = 20
    )]
    pub reconnect_grace_seconds: i64,

    /// Seconds between two comparisons of who is inside with LiveKit.
    #[arg(
        id = "spaces_reconcile_interval_seconds",
        long = "spaces-reconcile-interval-seconds",
        env = "TOKULY_SPACES_RECONCILE_INTERVAL_SECONDS",
        default_value_t = 20
    )]
    pub reconcile_interval_seconds: u64,

    /// Seconds an access token can be used to connect. Short on purpose:
    /// a token outlives a ban, and entering again hands out a fresh one.
    /// (A connected client is also given tokens by LiveKit itself, valid
    /// ten minutes; see `presence`.)
    #[arg(
        id = "spaces_token_ttl_seconds",
        long = "spaces-token-ttl-seconds",
        env = "TOKULY_SPACES_TOKEN_TTL_SECONDS",
        default_value_t = 120
    )]
    pub token_ttl_seconds: u64,

    /// How many days ahead a room can be scheduled.
    #[arg(
        id = "spaces_max_schedule_days",
        long = "spaces-max-schedule-days",
        env = "TOKULY_SPACES_MAX_SCHEDULE_DAYS",
        default_value_t = 30
    )]
    pub max_schedule_days: i64,

    /// Upcoming rooms one host can have at a time.
    #[arg(
        id = "spaces_max_scheduled",
        long = "spaces-max-scheduled",
        env = "TOKULY_SPACES_MAX_SCHEDULED",
        default_value_t = 10
    )]
    pub max_scheduled: i64,

    /// Hours after its planned start at which a room nobody started is
    /// cancelled.
    #[arg(
        id = "spaces_never_started_hours",
        long = "spaces-never-started-hours",
        env = "TOKULY_SPACES_NEVER_STARTED_HOURS",
        default_value_t = 24
    )]
    pub never_started_hours: i64,

    /// Days ended and cancelled rooms stay in their host's history.
    #[arg(
        id = "spaces_retention_days",
        long = "spaces-retention-days",
        env = "TOKULY_SPACES_RETENTION_DAYS",
        default_value_t = 90
    )]
    pub retention_days: i64,

    /// Days a room or hangout stays on record after its owner deleted it,
    /// and a former member's trace in a hangout after they left: time in
    /// which it can still be reported and moderators can look at it.
    #[arg(
        id = "spaces_evidence_days",
        long = "spaces-evidence-days",
        env = "TOKULY_SPACES_EVIDENCE_DAYS",
        default_value_t = 30
    )]
    pub evidence_days: i64,

    /// Seconds between two "your friend started a room" announcements of
    /// the same host. A friends room started sooner goes live quietly.
    #[arg(
        id = "spaces_started_announce_seconds",
        long = "spaces-started-announce-seconds",
        env = "TOKULY_SPACES_STARTED_ANNOUNCE_SECONDS",
        default_value_t = 1800
    )]
    pub started_announce_seconds: u64,

    /// Emoji bursts one person may send to a room in ten seconds.
    #[arg(
        id = "spaces_reactions_per_10s",
        long = "spaces-reactions-per-10s",
        env = "TOKULY_SPACES_REACTIONS_PER_10S",
        default_value_t = 20
    )]
    pub reactions_per_10s: u32,

    /// Hangouts one person may own.
    #[arg(
        id = "spaces_max_hangouts",
        long = "spaces-max-hangouts",
        env = "TOKULY_SPACES_MAX_HANGOUTS",
        default_value_t = 10
    )]
    pub max_hangouts: i64,

    /// Members of a hangout, open invitations included.
    #[arg(
        id = "spaces_max_hangout_members",
        long = "spaces-max-hangout-members",
        env = "TOKULY_SPACES_MAX_HANGOUT_MEMBERS",
        default_value_t = 50
    )]
    pub max_hangout_members: i64,

    /// Hangouts one person may be a member of.
    #[arg(
        id = "spaces_max_hangout_memberships",
        long = "spaces-max-hangout-memberships",
        env = "TOKULY_SPACES_MAX_HANGOUT_MEMBERSHIPS",
        default_value_t = 100
    )]
    pub max_hangout_memberships: i64,

    /// Seconds between two "someone is in the hangout" announcements of the
    /// same hangout.
    #[arg(
        id = "spaces_hangout_announce_seconds",
        long = "spaces-hangout-announce-seconds",
        env = "TOKULY_SPACES_HANGOUT_ANNOUNCE_SECONDS",
        default_value_t = 600
    )]
    pub hangout_announce_seconds: i64,
}

impl Default for SpacesConfig {
    fn default() -> Self {
        Self {
            max_speakers: 10,
            max_cohosts: 2,
            max_listeners: 3000,
            host_grace_seconds: 180,
            empty_grace_seconds: 120,
            connect_grace_seconds: 45,
            reconnect_grace_seconds: 20,
            reconcile_interval_seconds: 20,
            token_ttl_seconds: 120,
            max_schedule_days: 30,
            max_scheduled: 10,
            never_started_hours: 24,
            retention_days: 90,
            evidence_days: 30,
            started_announce_seconds: 1800,
            reactions_per_10s: 20,
            max_hangouts: 10,
            max_hangout_members: 50,
            max_hangout_memberships: 100,
            hangout_announce_seconds: 600,
        }
    }
}

impl SpacesConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_speakers >= 2,
            "TOKULY_SPACES_MAX_SPEAKERS must be at least 2 (the host and one guest)"
        );
        anyhow::ensure!(
            (0..self.max_speakers).contains(&self.max_cohosts),
            "TOKULY_SPACES_MAX_COHOSTS must be between 0 and the stage size minus the host"
        );
        anyhow::ensure!(
            self.max_listeners >= self.max_speakers,
            "TOKULY_SPACES_MAX_LISTENERS must be at least the stage size"
        );
        for (name, seconds) in [
            ("TOKULY_SPACES_HOST_GRACE_SECONDS", self.host_grace_seconds),
            ("TOKULY_SPACES_EMPTY_GRACE_SECONDS", self.empty_grace_seconds),
            ("TOKULY_SPACES_CONNECT_GRACE_SECONDS", self.connect_grace_seconds),
            ("TOKULY_SPACES_RECONNECT_GRACE_SECONDS", self.reconnect_grace_seconds),
            ("TOKULY_SPACES_HANGOUT_ANNOUNCE_SECONDS", self.hangout_announce_seconds),
        ] {
            anyhow::ensure!(seconds >= 0, "{name} must not be negative");
        }
        anyhow::ensure!(
            self.reconcile_interval_seconds >= 1,
            "TOKULY_SPACES_RECONCILE_INTERVAL_SECONDS must be at least 1"
        );
        anyhow::ensure!(self.token_ttl_seconds >= 30, "TOKULY_SPACES_TOKEN_TTL_SECONDS must be at least 30");
        anyhow::ensure!(self.max_schedule_days >= 1, "TOKULY_SPACES_MAX_SCHEDULE_DAYS must be at least 1");
        anyhow::ensure!(self.max_scheduled >= 1, "TOKULY_SPACES_MAX_SCHEDULED must be at least 1");
        anyhow::ensure!(self.never_started_hours >= 1, "TOKULY_SPACES_NEVER_STARTED_HOURS must be at least 1");
        anyhow::ensure!(self.retention_days >= 1, "TOKULY_SPACES_RETENTION_DAYS must be at least 1");
        anyhow::ensure!(self.evidence_days >= 1, "TOKULY_SPACES_EVIDENCE_DAYS must be at least 1");
        anyhow::ensure!(self.reactions_per_10s >= 1, "TOKULY_SPACES_REACTIONS_PER_10S must be at least 1");
        anyhow::ensure!(self.max_hangouts >= 1, "TOKULY_SPACES_MAX_HANGOUTS must be at least 1");
        anyhow::ensure!(self.max_hangout_members >= 2, "TOKULY_SPACES_MAX_HANGOUT_MEMBERS must be at least 2");
        anyhow::ensure!(
            self.max_hangout_memberships >= self.max_hangouts,
            "TOKULY_SPACES_MAX_HANGOUT_MEMBERSHIPS must be at least TOKULY_SPACES_MAX_HANGOUTS"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory as _, Parser};

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        spaces: SpacesConfig,
    }

    #[test]
    fn command_line_defaults_match_default_impl() {
        Cli::command().debug_assert();
        let parsed = Cli::parse_from(["test"]).spaces;
        assert_eq!(format!("{parsed:?}"), format!("{:?}", SpacesConfig::default()));
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn every_argument_is_namespaced() {
        for argument in Cli::command().get_arguments() {
            assert!(argument.get_id().as_str().starts_with("spaces_"), "{argument:?}");
            assert!(argument.get_long().unwrap().starts_with("spaces-"), "{argument:?}");
        }
    }

    #[test]
    fn nonsense_is_refused() {
        let ok = SpacesConfig::default();
        assert!(SpacesConfig { max_speakers: 1, ..ok.clone() }.validate().is_err());
        assert!(SpacesConfig { max_cohosts: 10, ..ok.clone() }.validate().is_err());
        assert!(SpacesConfig { max_listeners: 5, ..ok.clone() }.validate().is_err());
        assert!(SpacesConfig { host_grace_seconds: -1, ..ok.clone() }.validate().is_err());
        assert!(SpacesConfig { reconcile_interval_seconds: 0, ..ok.clone() }.validate().is_err());
        assert!(SpacesConfig { max_hangout_members: 1, ..ok }.validate().is_err());
    }
}
