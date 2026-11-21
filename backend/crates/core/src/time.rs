//! Conversions between `chrono` and protobuf timestamps.
use buffa::MessageField;
use buffa_types::google::protobuf::Timestamp;
use chrono::{DateTime, Utc};

pub type Time = DateTime<Utc>;

pub fn now() -> Time {
    Utc::now()
}

/// `DateTime` → set protobuf timestamp field.
pub fn ts<P: buffa::ProtoBox<Timestamp>>(time: Time) -> MessageField<Timestamp, P> {
    MessageField::some(Timestamp::from(time))
}

/// `Option<DateTime>` → protobuf timestamp field (unset when `None`).
pub fn opt_ts<P: buffa::ProtoBox<Timestamp>>(time: Option<Time>) -> MessageField<Timestamp, P> {
    match time {
        Some(time) => ts(time),
        None => MessageField::default(),
    }
}

/// Protobuf timestamp → `DateTime` (clamped to the representable range).
pub fn from_ts(ts: &Timestamp) -> Time {
    DateTime::from_timestamp(ts.seconds, ts.nanos.clamp(0, 999_999_999) as u32).unwrap_or_default()
}

/// Seconds + nanos pair (as exposed by timestamp views) → `DateTime`.
pub fn from_unix(seconds: i64, nanos: i32) -> Time {
    DateTime::from_timestamp(seconds, nanos.clamp(0, 999_999_999) as u32).unwrap_or_default()
}
