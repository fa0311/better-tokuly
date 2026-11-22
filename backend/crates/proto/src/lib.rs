//! Generated protobuf messages and Connect RPC service stubs.
//!
//! Everything lives under [`tokuly`], mirroring the proto packages:
//! `tokuly_proto::tokuly::identity::v1::AuthService`, etc. The short aliases
//! below are what the rest of the workspace imports.
#![allow(refining_impl_trait_internal, refining_impl_trait_reachable)]
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/_tokuly.rs"));

mod envelope;

/// Serialized `FileDescriptorSet` of the whole API (for gRPC reflection).
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/tokuly_descriptor.bin"));

pub use buffa;
pub use buffa_types;
pub use buffa_types::google::protobuf::Timestamp;

pub use tokuly::admin::v1 as admin;
pub use tokuly::ask::v1 as ask;
pub use tokuly::chat::v1 as chat;
pub use tokuly::common::v1 as common;
pub use tokuly::community::v1 as community;
pub use tokuly::dev::v1 as dev;
pub use tokuly::emoji::v1 as emoji;
pub use tokuly::engagement::v1 as engagement;
pub use tokuly::feed::v1 as feed;
pub use tokuly::geo::v1 as geo;
pub use tokuly::identity::v1 as identity;
pub use tokuly::live::v1 as live;
pub use tokuly::media::v1 as media;
pub use tokuly::notes::v1 as notes;
pub use tokuly::notify::v1 as notify;
pub use tokuly::plans::v1 as plans;
pub use tokuly::quest::v1 as quest;
pub use tokuly::realtime::v1 as realtime;
pub use tokuly::school::v1 as school;
pub use tokuly::search::v1 as search;
pub use tokuly::shorts::v1 as shorts;
pub use tokuly::social::v1 as social;
pub use tokuly::spaces::v1 as spaces;
pub use tokuly::stamp::v1 as stamp;
pub use tokuly::video::v1 as video;
pub use tokuly::wallet::v1 as wallet;
