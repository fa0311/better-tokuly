//! School-specific settings.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct SchoolConfig {
    /// Seconds without a heartbeat after which a study session (and a stay
    /// in a study room) is ended by the server. Time is only ever counted up
    /// to the last heartbeat, so this bounds how long a vanished client
    /// still looks like it is studying.
    #[arg(long = "school-session-idle-seconds", env = "TOKULY_SCHOOL_SESSION_IDLE_SECONDS", default_value_t = 900)]
    pub session_idle_seconds: i64,

    /// How often clients are asked to send a heartbeat, in seconds.
    #[arg(
        long = "school-heartbeat-interval-seconds",
        env = "TOKULY_SCHOOL_HEARTBEAT_INTERVAL_SECONDS",
        default_value_t = 60
    )]
    pub heartbeat_interval_seconds: i64,

    /// Seconds a session may stay paused before the server ends it.
    #[arg(
        long = "school-session-max-pause-seconds",
        env = "TOKULY_SCHOOL_SESSION_MAX_PAUSE_SECONDS",
        default_value_t = 10_800
    )]
    pub session_max_pause_seconds: i64,

    /// Longest a single session may last, in seconds (at most a day); the
    /// server ends it then.
    #[arg(long = "school-session-max-seconds", env = "TOKULY_SCHOOL_SESSION_MAX_SECONDS", default_value_t = 43_200)]
    pub session_max_seconds: i64,

    /// Seconds an empty study room stays open before it closes by itself.
    #[arg(
        long = "school-room-empty-close-seconds",
        env = "TOKULY_SCHOOL_ROOM_EMPTY_CLOSE_SECONDS",
        default_value_t = 600
    )]
    pub room_empty_close_seconds: i64,

    /// Member cap of a study room created without one (2-50).
    #[arg(
        long = "school-room-default-member-cap",
        env = "TOKULY_SCHOOL_ROOM_DEFAULT_MEMBER_CAP",
        default_value_t = 20
    )]
    pub room_default_member_cap: i32,

    /// Local hour (0-23) at which the "due tomorrow" assignment reminder is
    /// sent on the evening before.
    #[arg(
        long = "school-assignment-evening-hour",
        env = "TOKULY_SCHOOL_ASSIGNMENT_EVENING_HOUR",
        default_value_t = 19
    )]
    pub assignment_evening_hour: u32,

    /// Seconds between passes of the background sweep (idle sessions, study
    /// room phases, empty rooms, reminders).
    #[arg(
        id = "school_sweep_interval_seconds",
        long = "school-sweep-interval-seconds",
        env = "TOKULY_SCHOOL_SWEEP_INTERVAL_SECONDS",
        default_value_t = 5
    )]
    pub sweep_interval_seconds: u64,
}

impl Default for SchoolConfig {
    fn default() -> Self {
        Self {
            session_idle_seconds: 900,
            heartbeat_interval_seconds: 60,
            session_max_pause_seconds: 10_800,
            session_max_seconds: 43_200,
            room_empty_close_seconds: 600,
            room_default_member_cap: 20,
            assignment_evening_hour: 19,
            sweep_interval_seconds: 5,
        }
    }
}

impl SchoolConfig {
    /// Rejects settings the domain cannot run with.
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.session_idle_seconds >= 30, "TOKULY_SCHOOL_SESSION_IDLE_SECONDS must be at least 30");
        anyhow::ensure!(
            self.heartbeat_interval_seconds >= 5 && self.heartbeat_interval_seconds * 2 <= self.session_idle_seconds,
            "TOKULY_SCHOOL_HEARTBEAT_INTERVAL_SECONDS must be at least 5 and at most half the idle time"
        );
        anyhow::ensure!(
            self.session_max_pause_seconds >= 60,
            "TOKULY_SCHOOL_SESSION_MAX_PAUSE_SECONDS must be at least 60"
        );
        anyhow::ensure!(
            (600..=86_400).contains(&self.session_max_seconds),
            "TOKULY_SCHOOL_SESSION_MAX_SECONDS must be between 600 and 86400 (a day)"
        );
        anyhow::ensure!(
            self.room_empty_close_seconds >= 0,
            "TOKULY_SCHOOL_ROOM_EMPTY_CLOSE_SECONDS must not be negative"
        );
        anyhow::ensure!(
            (2..=crate::rooms::MAX_MEMBER_CAP).contains(&self.room_default_member_cap),
            "TOKULY_SCHOOL_ROOM_DEFAULT_MEMBER_CAP must be between 2 and {}",
            crate::rooms::MAX_MEMBER_CAP
        );
        anyhow::ensure!(
            self.assignment_evening_hour <= 23,
            "TOKULY_SCHOOL_ASSIGNMENT_EVENING_HOUR must be an hour of the day (0-23)"
        );
        anyhow::ensure!(self.sweep_interval_seconds >= 1, "TOKULY_SCHOOL_SWEEP_INTERVAL_SECONDS must be positive");
        Ok(())
    }
}
