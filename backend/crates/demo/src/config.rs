//! Settings of the simulator (`tokuly demo`): flags or `TOKULY_DEMO_*`.
use std::{path::PathBuf, str::FromStr};

use clap::Args;

use crate::town::{LatLng, Region};

#[derive(Debug, Clone, Args)]
pub struct DemoConfig {
    /// Where the API is reached.
    #[arg(long, env = "TOKULY_DEMO_API_URL", default_value = "http://127.0.0.1:8081")]
    pub api_url: String,

    /// E-mail addresses of the accounts that own the bots, comma-separated:
    /// the simulator signs them up on its first run. The server must name
    /// the same addresses in its own TOKULY_DEV_DEMO_OPERATORS, or the bots
    /// are refused the right to move, chat and post. One account owns five
    /// bots unless the server says otherwise (TOKULY_DEV_MAX_BOTS).
    #[arg(long, env = "TOKULY_DEV_DEMO_OPERATORS", value_delimiter = ',', default_value = "")]
    pub operators: Vec<String>,

    /// Password of the operator accounts (at least 16 characters). The server
    /// knows it too: it is how the server tells the demo's accounts from
    /// someone who signed up with one of their addresses.
    #[arg(long, env = "TOKULY_DEV_DEMO_PASSWORD", default_value = "", hide_env_values = true)]
    pub password: String,

    /// The same seed tells the same story: who the bots are, where they go
    /// and what they say.
    #[arg(long, env = "TOKULY_DEMO_SEED", default_value_t = 2026)]
    pub seed: u64,

    /// Which episode of the seed's story to play. The cast and the town
    /// stay the same from episode to episode; the walks, posts and
    /// conversations differ. Unset, it is the second the simulator starts
    /// in, so a restart (even a quick one after a crash) does not post the
    /// same notes again; set it to play
    /// exactly the same demo twice.
    #[arg(long, env = "TOKULY_DEMO_EPISODE")]
    pub episode: Option<u64>,

    /// How many bots take part.
    #[arg(long, env = "TOKULY_DEMO_BOTS", default_value_t = 12)]
    pub bots: usize,

    /// Middle of the region the bots move in, as `latitude,longitude`. It
    /// should be inside the area the map shows at street level
    /// (TOKULY_MAP_REGION_BBOX).
    #[arg(long, env = "TOKULY_DEMO_CENTER", default_value = "35.7030,139.5795")]
    pub center: Center,

    /// Radius of that region in meters.
    #[arg(long, env = "TOKULY_DEMO_RADIUS_M", default_value_t = 900.0)]
    pub radius_m: f64,

    /// How fast the demo's clock runs: 1 is real time, 2 makes everyone
    /// walk and post twice as fast.
    #[arg(long, env = "TOKULY_DEMO_PACE", default_value_t = 1.0)]
    pub pace: f64,

    /// How many more bots ask a visitor to be friends once the visitor has
    /// befriended one of them.
    #[arg(long, env = "TOKULY_DEMO_COMPANIONS", default_value_t = 3)]
    pub companions: usize,

    /// Send uploads to this address instead of the one in the signed upload
    /// URL, which names the object store as browsers reach it
    /// (TOKULY_S3_PUBLIC_ENDPOINT). For a simulator that runs next to the
    /// store and cannot reach that address, as in Docker Compose.
    #[arg(long, env = "TOKULY_DEMO_STORAGE_URL")]
    pub storage_url: Option<String>,

    /// Where the bots' API tokens are kept between starts, sealed with the
    /// password: an operator may make only a few tokens an hour. Unset, it is
    /// `tokuly-demo` in the user's state directory (`$XDG_STATE_HOME`,
    /// `~/.local/state`).
    #[arg(long, env = "TOKULY_DEMO_STATE_DIR")]
    pub state_dir: Option<PathBuf>,

    /// Stop after this many seconds (0 = run until stopped). For trying it out.
    #[arg(long, env = "TOKULY_DEMO_RUN_SECONDS", default_value_t = 0)]
    pub run_seconds: u64,
}

/// A coordinate written as `latitude,longitude`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Center(pub LatLng);

impl FromStr for Center {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let number = |part: &str| part.trim().parse::<f64>().ok();
        match text.split_once(',').map(|(lat, lng)| (number(lat), number(lng))) {
            Some((Some(lat), Some(lng))) if lat.abs() <= 85.0 && lng.abs() <= 180.0 => Ok(Self(LatLng { lat, lng })),
            _ => Err("write it as latitude,longitude, for example 35.7030,139.5795".to_owned()),
        }
    }
}

impl DemoConfig {
    /// The operator addresses as given, without blanks.
    pub fn operators(&self) -> Vec<&str> {
        self.operators.iter().map(|address| address.trim()).filter(|address| !address.is_empty()).collect()
    }

    /// The episode to play: the one asked for, or the current second's.
    pub fn episode(&self) -> u64 {
        let seconds =
            || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs());
        self.episode.unwrap_or_else(seconds)
    }

    /// Where the bots' tokens are kept, if anywhere.
    pub fn state_dir(&self) -> Option<PathBuf> {
        self.state_dir.clone().filter(|dir| !dir.as_os_str().is_empty()).or_else(crate::keep::default_dir)
    }

    pub fn region(&self) -> Region {
        Region { center: self.center.0, radius_m: self.radius_m }
    }

    /// Refuses settings the simulator cannot run with, saying what to set.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.operators().is_empty(),
            "the demo is off: set TOKULY_DEV_DEMO_OPERATORS (e-mail addresses for the accounts that own the bots) for the server and for the simulator"
        );
        anyhow::ensure!(
            self.password.chars().count() >= 16,
            "set TOKULY_DEV_DEMO_PASSWORD (at least 16 characters, the same for the server): the password of the operator accounts"
        );
        anyhow::ensure!(self.bots >= 1, "TOKULY_DEMO_BOTS must be at least 1");
        anyhow::ensure!(
            (50.0..=20_000.0).contains(&self.radius_m),
            "TOKULY_DEMO_RADIUS_M must be between 50 and 20000"
        );
        anyhow::ensure!(self.pace > 0.0 && self.pace <= 20.0, "TOKULY_DEMO_PACE must be above 0 and at most 20");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct CommandLine {
        #[command(flatten)]
        demo: DemoConfig,
    }

    fn parse(flags: &[&str]) -> DemoConfig {
        CommandLine::try_parse_from(std::iter::once("demo").chain(flags.iter().copied())).unwrap().demo
    }

    #[test]
    fn without_operators_and_a_password_the_demo_is_off() {
        // (The environment of a test run names neither.)
        let off = parse(&[]);
        assert!(off.validate().unwrap_err().to_string().contains("the demo is off"));
        let half = parse(&["--operators", "a@demo.example, ,b@demo.example"]);
        assert_eq!(half.operators(), ["a@demo.example", "b@demo.example"]);
        assert!(half.validate().unwrap_err().to_string().contains("TOKULY_DEV_DEMO_PASSWORD"));
        parse(&["--operators", "a@demo.example", "--password", "long enough, really"]).validate().unwrap();
    }

    #[test]
    fn the_center_is_a_coordinate() {
        assert_eq!("35.5, 139.25".parse(), Ok(Center(LatLng { lat: 35.5, lng: 139.25 })));
        for wrong in ["", "35.5", "north,east", "95,10", "10,190"] {
            assert!(wrong.parse::<Center>().is_err(), "{wrong}");
        }
        assert!(CommandLine::try_parse_from(["demo", "--center", "here"]).is_err());
    }

    #[test]
    fn the_episode_is_the_one_asked_for_or_the_second() {
        assert_eq!(parse(&["--episode", "7"]).episode(), 7);
        // Seconds since 1970: past 2025, and (nearly) the same when asked twice in a row.
        let now = parse(&[]).episode();
        assert!(now > 1_735_000_000 && now.abs_diff(parse(&[]).episode()) <= 1);
    }

    #[test]
    fn limits_are_checked() {
        let base = ["--operators", "a@demo.example", "--password", "long enough, really"];
        for wrong in [["--bots", "0"], ["--radius-m", "5"], ["--pace", "0"], ["--pace", "50"]] {
            assert!(parse(&[&base[..], &wrong[..]].concat()).validate().is_err(), "{wrong:?}");
        }
    }
}
