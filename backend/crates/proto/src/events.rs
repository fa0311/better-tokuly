//! Which messages may travel as realtime events.
//!
//! The gateway's envelope carries any encoded message, but clients only
//! understand the event messages listed at `Event` in `realtime.proto`.
//! [`RealtimeEvent`] is implemented for exactly those, so that publishing
//! anything else (a message of the domain, a value of the event's `oneof`)
//! does not compile instead of being dropped by every client.
use buffa::{Message, MessageName};

mod sealed {
    pub trait Sealed {}
}

/// A domain's event message, as listed in `realtime.proto`. Implemented
/// here and nowhere else: a domain that gets an event message adds it to
/// the list below, to the list in `realtime.proto` and to
/// `packages/realtime/src/events.ts` (tests keep the three in step).
pub trait RealtimeEvent: Message + MessageName + sealed::Sealed {}

macro_rules! realtime_events {
    ($($feature:literal => $package:ident::$event:ident,)*) => {
        $(
            #[cfg(feature = $feature)]
            impl sealed::Sealed for crate::$package::$event {}
            #[cfg(feature = $feature)]
            impl RealtimeEvent for crate::$package::$event {}
        )*
    };
}

// One line per event message: its package (the feature) and its name.
realtime_events! {
    "geo" => geo::GeoEvent,
    "social" => social::SocialEvent,
    "chat" => chat::ChatEvent,
    "notify" => notify::NotifyEvent,
    "live" => live::LiveEvent,
    "video" => video::VideoEvent,
    "notes" => notes::NotesEvent,
    "media" => media::MediaEvent,
    "wallet" => wallet::WalletEvent,
    "engagement" => engagement::EngagementEvent,
    "quest" => quest::QuestEvent,
    "plans" => plans::PlansEvent,
    "school" => school::SchoolEvent,
    "spaces" => spaces::SpacesEvent,
    "community" => community::CommunityEvent,
    "ask" => ask::AskEvent,
    "emoji" => emoji::EmojiEvent,
    "moments" => moments::MomentsEvent,
}
