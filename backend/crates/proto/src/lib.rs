//! Generated protobuf messages and Connect RPC service stubs.
//!
//! Every proto package is a module behind a Cargo feature of the same name:
//! with the `wallet` feature, `tokuly_proto::wallet` is the package
//! `tokuly.wallet.v1` (`proto/tokuly/wallet`). A crate enables the packages
//! it names in its own code, so it compiles only the part of the API it uses
//! and is rebuilt only when that part changes:
//!
//! ```toml
//! tokuly-proto = { workspace = true, features = ["common", "wallet"] }
//! ```
//!
//! The generated code lives under [`tokuly`], mirroring the proto packages
//! (`tokuly_proto::tokuly::identity::v1::AuthService`); the short aliases
//! below are what the rest of the workspace imports. The `client` feature
//! adds the generated Connect clients.
#![allow(refining_impl_trait_internal, refining_impl_trait_reachable)]
#![allow(clippy::all)]

// The modules of the enabled packages (see `build.rs`).
include!(concat!(env!("OUT_DIR"), "/_tokuly.rs"));

#[cfg(feature = "realtime")]
mod envelope;
mod events;

pub use events::RealtimeEvent;

/// Serialized `FileDescriptorSet` of the enabled packages and what they
/// import (for gRPC reflection).
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/tokuly_descriptor.bin"));

pub use buffa;
pub use buffa_types;
pub use buffa_types::google::protobuf::Timestamp;

#[cfg(feature = "admin")]
pub use tokuly::admin::v1 as admin;
#[cfg(feature = "ask")]
pub use tokuly::ask::v1 as ask;
#[cfg(feature = "chat")]
pub use tokuly::chat::v1 as chat;
#[cfg(feature = "common")]
pub use tokuly::common::v1 as common;
#[cfg(feature = "community")]
pub use tokuly::community::v1 as community;
#[cfg(feature = "dev")]
pub use tokuly::dev::v1 as dev;
#[cfg(feature = "emoji")]
pub use tokuly::emoji::v1 as emoji;
#[cfg(feature = "engagement")]
pub use tokuly::engagement::v1 as engagement;
#[cfg(feature = "feed")]
pub use tokuly::feed::v1 as feed;
#[cfg(feature = "geo")]
pub use tokuly::geo::v1 as geo;
#[cfg(feature = "identity")]
pub use tokuly::identity::v1 as identity;
#[cfg(feature = "live")]
pub use tokuly::live::v1 as live;
#[cfg(feature = "media")]
pub use tokuly::media::v1 as media;
#[cfg(feature = "moments")]
pub use tokuly::moments::v1 as moments;
#[cfg(feature = "notes")]
pub use tokuly::notes::v1 as notes;
#[cfg(feature = "notify")]
pub use tokuly::notify::v1 as notify;
#[cfg(feature = "plans")]
pub use tokuly::plans::v1 as plans;
#[cfg(feature = "quest")]
pub use tokuly::quest::v1 as quest;
#[cfg(feature = "realtime")]
pub use tokuly::realtime::v1 as realtime;
#[cfg(feature = "school")]
pub use tokuly::school::v1 as school;
#[cfg(feature = "search")]
pub use tokuly::search::v1 as search;
#[cfg(feature = "shorts")]
pub use tokuly::shorts::v1 as shorts;
#[cfg(feature = "social")]
pub use tokuly::social::v1 as social;
#[cfg(feature = "spaces")]
pub use tokuly::spaces::v1 as spaces;
#[cfg(feature = "stamp")]
pub use tokuly::stamp::v1 as stamp;
#[cfg(feature = "video")]
pub use tokuly::video::v1 as video;
#[cfg(feature = "wallet")]
pub use tokuly::wallet::v1 as wallet;
