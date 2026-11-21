//! Location ingest, ghost mode, places, visits, footprints, trips and safety.
use std::{ops::Deref, sync::Arc};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;

pub mod config;
mod ctx;
pub mod detect;
pub mod geocode;
pub mod math;
pub mod odometer;
mod state;
pub mod visibility;

pub use config::GeoConfig;

pub struct GeoInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: GeoConfig,
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
