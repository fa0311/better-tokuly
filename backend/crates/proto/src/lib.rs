//! Generated protobuf messages and Connect RPC service stubs.
//!
//! Everything lives under [`tokuly`], mirroring the proto packages:
//! `tokuly_proto::tokuly::identity::v1::AuthService`, etc. The short aliases
//! below are what the rest of the workspace imports.
#![allow(refining_impl_trait_internal, refining_impl_trait_reachable)]
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/_tokuly.rs"));

/// Serialized `FileDescriptorSet` of the whole API (for gRPC reflection).
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/tokuly_descriptor.bin"));

pub use buffa;
pub use buffa_types;
pub use buffa_types::google::protobuf::Timestamp;

pub use tokuly::admin::v1 as admin;
pub use tokuly::chat::v1 as chat;
pub use tokuly::common::v1 as common;
pub use tokuly::engagement::v1 as engagement;
pub use tokuly::feed::v1 as feed;
pub use tokuly::geo::v1 as geo;
pub use tokuly::identity::v1 as identity;
pub use tokuly::live::v1 as live;
pub use tokuly::media::v1 as media;
pub use tokuly::notes::v1 as notes;
pub use tokuly::notify::v1 as notify;
pub use tokuly::realtime::v1 as realtime;
pub use tokuly::search::v1 as search;
pub use tokuly::shorts::v1 as shorts;
pub use tokuly::social::v1 as social;
pub use tokuly::stamp::v1 as stamp;
pub use tokuly::video::v1 as video;
pub use tokuly::wallet::v1 as wallet;
