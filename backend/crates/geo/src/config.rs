//! Geo-specific settings.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct GeoConfig {
    /// Base URL of a Photon geocoder (`/api` and `/reverse`) used for place
    /// search and the `ReverseGeocode` RPC. Empty = the bundled offline
    /// gazetteer only.
    #[arg(id = "geo_geocoder_url", long = "geocoder-url", env = "TOKULY_GEOCODER_URL", default_value = "")]
    pub geocoder_url: String,

    /// Base URL of a Valhalla routing engine (`/route`) for directions and
    /// trip plans. Empty = great-circle estimates with typical speeds.
    #[arg(id = "geo_routing_url", long = "routing-url", env = "TOKULY_ROUTING_URL", default_value = "")]
    pub routing_url: String,

    /// Base URL of an Open-Meteo compatible weather API (`/v1/forecast`).
    /// Empty = no weather on friends' pins.
    #[arg(id = "geo_weather_url", long = "weather-url", env = "TOKULY_WEATHER_URL", default_value = "")]
    pub weather_url: String,

    /// A user is "staying" once they have been within this many meters...
    #[arg(
        id = "geo_stay_radius_meters",
        long = "geo-stay-radius-meters",
        env = "TOKULY_GEO_STAY_RADIUS_METERS",
        default_value_t = 80.0
    )]
    pub stay_radius_meters: f64,
    /// ...for this many seconds.
    #[arg(id = "geo_stay_seconds", long = "geo-stay-seconds", env = "TOKULY_GEO_STAY_SECONDS", default_value_t = 300)]
    pub stay_seconds: i64,

    /// Friends form a gathering once they have been within this many meters of each other...
    #[arg(
        id = "geo_gathering_radius_meters",
        long = "geo-gathering-radius-meters",
        env = "TOKULY_GEO_GATHERING_RADIUS_METERS",
        default_value_t = 30.0
    )]
    pub gathering_radius_meters: f64,
    /// ...for this many seconds.
    #[arg(
        id = "geo_gathering_seconds",
        long = "geo-gathering-seconds",
        env = "TOKULY_GEO_GATHERING_SECONDS",
        default_value_t = 120
    )]
    pub gathering_seconds: i64,

    /// Upload interval suggested to clients while moving.
    #[arg(
        id = "geo_moving_interval_seconds",
        long = "geo-moving-interval-seconds",
        env = "TOKULY_GEO_MOVING_INTERVAL_SECONDS",
        default_value_t = 5
    )]
    pub moving_interval_seconds: i32,
    /// Upload interval suggested to clients while still.
    #[arg(
        id = "geo_stationary_interval_seconds",
        long = "geo-stationary-interval-seconds",
        env = "TOKULY_GEO_STATIONARY_INTERVAL_SECONDS",
        default_value_t = 300
    )]
    pub stationary_interval_seconds: i32,

    /// Samples recorded longer ago than this are not accepted (backlog uploads).
    #[arg(
        id = "geo_max_sample_age_hours",
        long = "geo-max-sample-age-hours",
        env = "TOKULY_GEO_MAX_SAMPLE_AGE_HOURS",
        default_value_t = 168
    )]
    pub max_sample_age_hours: i64,

    /// Nights at the same unlabelled spot before it is suggested as home.
    #[arg(id = "geo_home_nights", long = "geo-home-nights", env = "TOKULY_GEO_HOME_NIGHTS", default_value_t = 3)]
    pub home_nights: i64,

    /// A one-time place alert that has not fired lapses after this many hours.
    #[arg(
        id = "geo_one_time_alert_hours",
        long = "geo-one-time-alert-hours",
        env = "TOKULY_GEO_ONE_TIME_ALERT_HOURS",
        default_value_t = 24
    )]
    pub one_time_alert_hours: i64,
    /// A recurring place alert waiting for approval lapses after this many days.
    #[arg(
        id = "geo_pending_alert_days",
        long = "geo-pending-alert-days",
        env = "TOKULY_GEO_PENDING_ALERT_DAYS",
        default_value_t = 7
    )]
    pub pending_alert_days: i64,

    /// An SOS nobody resolved ends by itself after this many hours (ghost mode applies again).
    #[arg(
        id = "geo_sos_max_hours",
        long = "geo-sos-max-hours",
        env = "TOKULY_GEO_SOS_MAX_HOURS",
        default_value_t = 24
    )]
    pub sos_max_hours: i64,

    /// Most cells `GetCells` returns for one viewport.
    #[arg(id = "geo_max_cells", long = "geo-max-cells", env = "TOKULY_GEO_MAX_CELLS", default_value_t = 20_000)]
    pub max_cells: i64,

    /// Location samples from the realtime gateway processed in parallel by
    /// one worker process (samples of one user always go through one lane).
    #[arg(
        id = "geo_pipeline_lanes",
        long = "geo-pipeline-lanes",
        env = "TOKULY_GEO_PIPELINE_LANES",
        default_value_t = 16
    )]
    pub pipeline_lanes: usize,
}

impl Default for GeoConfig {
    fn default() -> Self {
        Self {
            geocoder_url: String::new(),
            routing_url: String::new(),
            weather_url: String::new(),
            stay_radius_meters: 80.0,
            stay_seconds: 300,
            gathering_radius_meters: 30.0,
            gathering_seconds: 120,
            moving_interval_seconds: 5,
            stationary_interval_seconds: 300,
            max_sample_age_hours: 168,
            home_nights: 3,
            one_time_alert_hours: 24,
            pending_alert_days: 7,
            sos_max_hours: 24,
            max_cells: 20_000,
            pipeline_lanes: 16,
        }
    }
}

impl GeoConfig {
    /// Rejects settings the domain cannot run with.
    pub fn validate(&self) -> anyhow::Result<()> {
        for (name, meters) in [
            ("TOKULY_GEO_STAY_RADIUS_METERS", self.stay_radius_meters),
            ("TOKULY_GEO_GATHERING_RADIUS_METERS", self.gathering_radius_meters),
        ] {
            anyhow::ensure!(meters.is_finite() && meters > 0.0, "{name} must be a positive number of meters");
        }
        for (name, value) in [
            ("TOKULY_GEO_STAY_SECONDS", self.stay_seconds),
            ("TOKULY_GEO_GATHERING_SECONDS", self.gathering_seconds),
            ("TOKULY_GEO_MAX_SAMPLE_AGE_HOURS", self.max_sample_age_hours),
            ("TOKULY_GEO_HOME_NIGHTS", self.home_nights),
            ("TOKULY_GEO_ONE_TIME_ALERT_HOURS", self.one_time_alert_hours),
            ("TOKULY_GEO_PENDING_ALERT_DAYS", self.pending_alert_days),
            ("TOKULY_GEO_SOS_MAX_HOURS", self.sos_max_hours),
            ("TOKULY_GEO_MAX_CELLS", self.max_cells),
            ("TOKULY_GEO_MOVING_INTERVAL_SECONDS", self.moving_interval_seconds as i64),
            ("TOKULY_GEO_STATIONARY_INTERVAL_SECONDS", self.stationary_interval_seconds as i64),
        ] {
            anyhow::ensure!(value > 0, "{name} must be positive");
        }
        anyhow::ensure!(self.pipeline_lanes > 0, "TOKULY_GEO_PIPELINE_LANES must be positive");
        for (name, url) in [
            ("TOKULY_GEOCODER_URL", &self.geocoder_url),
            ("TOKULY_ROUTING_URL", &self.routing_url),
            ("TOKULY_WEATHER_URL", &self.weather_url),
        ] {
            anyhow::ensure!(
                url.is_empty() || url.starts_with("http://") || url.starts_with("https://"),
                "{name} must be an http(s) URL"
            );
        }
        Ok(())
    }
}

impl GeoConfig {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "stay_radius_meters",
        "stay_seconds",
        "gathering_radius_meters",
        "gathering_seconds",
        "moving_interval_seconds",
        "stationary_interval_seconds",
        "max_sample_age_hours",
        "home_nights",
        "one_time_alert_hours",
        "pending_alert_days",
        "sos_max_hours",
        "max_cells",
        "pipeline_lanes",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_bad_values_are_rejected() {
        assert!(GeoConfig::default().validate().is_ok());
        assert!(GeoConfig { stay_radius_meters: 0.0, ..Default::default() }.validate().is_err());
        assert!(GeoConfig { gathering_seconds: 0, ..Default::default() }.validate().is_err());
        assert!(GeoConfig { pipeline_lanes: 0, ..Default::default() }.validate().is_err());
        assert!(GeoConfig { routing_url: "valhalla:8002".into(), ..Default::default() }.validate().is_err());
        assert!(GeoConfig { routing_url: "http://valhalla:8002".into(), ..Default::default() }.validate().is_ok());
    }
}
