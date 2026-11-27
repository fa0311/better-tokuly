//! Location sharing: the live friend map, ghost mode, places, visit history,
//! footprints, trips, place alerts, SOS and live shares.
//!
//! # How a location sample travels
//!
//! ```text
//! device ──▶ realtime gateway ──▶ LOCATIONS stream ──┐
//! device ──▶ LocationService.ReportLocations ────────┤
//!                                                    ▼
//!                                  pipeline (per user, in order, under a lock)
//!        ┌──────────────┬──────────────┬───────────┴───┬───────────────┬──────────────┐
//!   live state      stays → visits   footprints    raw history     alerts, trip,   pin to each friend
//!   (Valkey) and    places, nights   (PostGIS)     (ClickHouse)    gatherings      through ghost mode
//!   core geoindex   (PostgreSQL)
//! ```
//!
//! # Layout
//!
//! - Pure logic, unit-tested without infrastructure: [`detect`] (stays,
//!   proximity, activity, sleep, nights), [`odometer`], [`visibility`] (what a
//!   viewer sees: precise / blurred / frozen / SOS), [`suggest`], [`math`],
//!   [`geocode`] (offline gazetteer and time zones), [`routing`] estimates.
//! - `pipeline` ties them to storage; `state` and `ctx` are what it reads and
//!   writes per user; `fanout` shows people each other's pins.
//! - One module per feature with its RPC handlers: `location_svc`,
//!   `place_svc` (+ `places`), `history_svc` (+ `history`, `visits`),
//!   `footprints` (+ `regions`), `trips`, `safety`, plus `ghost`, `shares`
//!   and `gatherings`.
//! - `consumers` reacts to other domains' events, `workers` runs the
//!   background loops, `hooks` plugs into profiles, subjects and exports.
//!
//! # For other domains
//!
//! Positions for rule checks ("are these two within 200 m?") are in
//! `tokuly_core::geoindex`, which this crate keeps up to date. This crate's
//! own public API is small: [`Geo::ingest`] (feed a sample directly),
//! [`Geo::seed_regions`] (load the bundled boundaries; call at startup),
//! [`Geo::handle_event`], the check of positions claimed in the social
//! domain (`claims`, plugged into social by [`Geo::new`]), and the
//! maintenance entry points
//! ([`Geo::sweep`], [`Geo::enforce_retention`], [`Geo::flush_history`]).
use std::{ops::Deref, sync::Arc, time::Duration};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::common::SubjectType;
use tokuly_social::Social;

mod claims;
pub mod config;
mod consumers;
mod ctx;
pub mod detect;
mod fanout;
mod footprints;
mod gatherings;
pub mod geocode;
mod ghost;
mod history;
mod history_svc;
mod hooks;
mod location_svc;
pub mod math;
pub mod odometer;
mod pipeline;
mod place_svc;
mod places;
mod regions;
pub mod routing;
mod safety;
mod shares;
mod state;
pub mod suggest;
mod trips;
pub mod visibility;
mod visits;
mod weather;
mod workers;

pub use config::GeoConfig;
pub use history::CLICKHOUSE_SCHEMA;
pub use regions::WORLD_ID;

/// The columns this domain seals with the server secret (`tokuly rekey`).
pub const SEALED: &[tokuly_core::rekey::Sealed] = &[shares::SEALED];

pub struct GeoInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: GeoConfig,
    /// Keyed hash behind blurred positions (derived from the server secret).
    blur_mac: Box<dyn Fn(&[u8]) -> [u8; 32] + Send + Sync>,
    /// Raw history rows waiting to be written to ClickHouse.
    history: history::Buffer,
    regions: regions::Regions,
    /// Whether an account's language is Japanese, cached briefly.
    locales: moka::sync::Cache<Uuid, bool>,
}

/// The geo domain. Cheap to clone.
#[derive(Clone)]
pub struct Geo(Arc<GeoInner>);

impl Deref for Geo {
    type Target = GeoInner;

    fn deref(&self) -> &GeoInner {
        &self.0
    }
}

impl Geo {
    /// Builds the domain and plugs it into the others: profiles gain the
    /// distance to a friend and footprint numbers, places become subjects
    /// (comments, reactions, reports), the personal data export gains a
    /// `geo` section, and the social domain checks the positions clients
    /// claim for nearby discovery and bumps against this domain's fixes.
    pub fn new(core: Core, identity: Identity, cfg: GeoConfig, social: &Social) -> anyhow::Result<Self> {
        cfg.validate()?;
        let geo = Self(Arc::new(GeoInner {
            blur_mac: Geo::make_blur_mac(&core),
            history: history::Buffer::default(),
            regions: regions::Regions::default(),
            locales: moka::sync::Cache::builder().max_capacity(200_000).time_to_live(Duration::from_secs(300)).build(),
            core: core.clone(),
            identity: identity.clone(),
            cfg,
        }));
        identity.add_profile_extension(Arc::new(geo.clone()));
        core.subjects.register(SubjectType::SUBJECT_TYPE_PLACE, Arc::new(hooks::PlaceSubjects(geo.clone())));
        core.exporters.register(Arc::new(hooks::GeoExporter(geo.clone())));
        social.set_position_check(Arc::new(geo.clone()));
        // The gazetteer takes a moment to load: do it before the first request needs it.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(|| {
                geocode::Gazetteer::shared();
            });
        }
        Ok(geo)
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::geo::{
            FootprintServiceExt, HistoryServiceExt, LocationServiceExt, PlaceServiceExt, SafetyServiceExt,
            TripServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = LocationServiceExt::register(me.clone(), router);
        let router = PlaceServiceExt::register(me.clone(), router);
        let router = HistoryServiceExt::register(me.clone(), router);
        let router = FootprintServiceExt::register(me.clone(), router);
        let router = TripServiceExt::register(me.clone(), router);
        SafetyServiceExt::register(me, router)
    }
}
