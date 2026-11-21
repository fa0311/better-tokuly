//! Building the gateway's own frames and classifying frames it forwards.
//!
//! Frames published by the domains arrive already encoded and are forwarded
//! verbatim. The only thing the gateway needs to know about them is whether
//! losing one is acceptable; [`delivery_of`] finds that out by reading a few
//! tag bytes instead of decoding the whole message.
use buffa::Message as _;
use bytes::Bytes;
use tokuly_core::time::{now, ts};
use tokuly_proto::realtime::{Error, Pong, ServerFrame, TopicStats, Welcome, server_frame::Body};

/// How a frame may be treated on its way to a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// Must arrive, in order. A client that cannot keep up is disconnected.
    Reliable,
    /// Superseded by the next one of its kind (a friend's position): travels
    /// as a datagram where possible and the oldest is dropped under pressure.
    Droppable,
}

/// Connect-style error codes used in `Error.code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Code {
    InvalidArgument,
    Unauthenticated,
    PermissionDenied,
    FailedPrecondition,
    ResourceExhausted,
    Unavailable,
}

impl Code {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Code::InvalidArgument => "invalid_argument",
            Code::Unauthenticated => "unauthenticated",
            Code::PermissionDenied => "permission_denied",
            Code::FailedPrecondition => "failed_precondition",
            Code::ResourceExhausted => "resource_exhausted",
            Code::Unavailable => "unavailable",
        }
    }
}

/// A rejected client frame: the code plus a machine-readable reason that
/// travels as `Error.message` (e.g. `TOKEN_EXPIRED`, `TOPIC_INVALID`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rejection {
    pub code: Code,
    pub reason: &'static str,
}

impl Rejection {
    pub(crate) const fn new(code: Code, reason: &'static str) -> Self {
        Self { code, reason }
    }
}

fn encode(reply_to: u64, body: Body) -> Bytes {
    let frame = ServerFrame { reply_to, body: Some(body), ..Default::default() };
    Bytes::from(frame.encode_to_vec())
}

pub(crate) fn welcome(reply_to: u64, welcome: Welcome) -> Bytes {
    encode(reply_to, Body::Welcome(Box::new(welcome)))
}

pub(crate) fn pong(reply_to: u64, nonce: u64) -> Bytes {
    encode(reply_to, Body::Pong(Box::new(Pong { nonce, server_time: ts(now()), ..Default::default() })))
}

pub(crate) fn ack(reply_to: u64) -> Bytes {
    encode(reply_to, Body::Ack(Box::default()))
}

pub(crate) fn error(reply_to: u64, rejection: Rejection, fatal: bool) -> Bytes {
    let error = Error { code: rejection.code.as_str().to_owned(), message: rejection.reason.to_owned(), fatal, ..Default::default() };
    encode(reply_to, Body::Error(Box::new(error)))
}

/// The body of a `TopicStats` push (published through the normal topic path).
pub(crate) fn topic_stats(subscriber_count: i64) -> Body {
    Body::TopicStats(Box::new(TopicStats { subscriber_count, ..Default::default() }))
}

// Field numbers the classifier looks for. `classification_follows_the_schema`
// below fails if the proto files are renumbered without updating these.
const SERVER_FRAME_GEO: u32 = 10;
const GEO_EVENT_LOCATION: u32 = 1;
const WIRE_VARINT: u8 = 0;
const WIRE_FIXED64: u8 = 1;
const WIRE_LEN: u8 = 2;
const WIRE_FIXED32: u8 = 5;

/// Decides how an encoded `ServerFrame` may be delivered. Only a friend's
/// position (`geo.location`) is droppable; anything unreadable is treated as
/// reliable so that nothing is ever lost by mistake.
pub(crate) fn delivery_of(frame: &[u8]) -> Delivery {
    match geo_event(frame).and_then(first_field_number) {
        Some(GEO_EVENT_LOCATION) => Delivery::Droppable,
        _ => Delivery::Reliable,
    }
}

/// The encoded `GeoEvent` inside a `ServerFrame`, if that is its body.
fn geo_event(mut frame: &[u8]) -> Option<&[u8]> {
    while !frame.is_empty() {
        let (field, wire) = read_tag(&mut frame)?;
        if field == SERVER_FRAME_GEO && wire == WIRE_LEN {
            return read_len_delimited(&mut frame);
        }
        skip_value(&mut frame, wire)?;
    }
    None
}

fn first_field_number(mut message: &[u8]) -> Option<u32> {
    read_tag(&mut message).map(|(field, _)| field)
}

fn read_varint(buf: &mut &[u8]) -> Option<u64> {
    let mut value = 0u64;
    for index in 0..10 {
        let byte = *buf.get(index)?;
        value |= u64::from(byte & 0x7f) << (7 * index);
        if byte & 0x80 == 0 {
            *buf = &buf[index + 1..];
            return Some(value);
        }
    }
    None
}

fn read_tag(buf: &mut &[u8]) -> Option<(u32, u8)> {
    let tag = read_varint(buf)?;
    Some((u32::try_from(tag >> 3).ok()?, (tag & 0x7) as u8))
}

fn read_len_delimited<'a>(buf: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = usize::try_from(read_varint(buf)?).ok()?;
    let current: &'a [u8] = buf;
    let value = current.get(..len)?;
    *buf = &current[len..];
    Some(value)
}

fn skip_value(buf: &mut &[u8], wire: u8) -> Option<()> {
    match wire {
        WIRE_VARINT => read_varint(buf).map(drop),
        WIRE_LEN => read_len_delimited(buf).map(drop),
        WIRE_FIXED64 => skip_bytes(buf, 8),
        WIRE_FIXED32 => skip_bytes(buf, 4),
        _ => None,
    }
}

fn skip_bytes(buf: &mut &[u8], count: usize) -> Option<()> {
    let current: &[u8] = buf;
    *buf = current.get(count..)?;
    Some(())
}

#[cfg(test)]
mod tests {
    use tokuly_proto::{
        common::LatLng,
        geo::{FriendLocation, GeoEvent, geo_event::Event},
    };

    use super::*;

    fn frame(reply_to: u64, topic: &str, body: Body) -> Vec<u8> {
        ServerFrame { reply_to, topic: topic.to_owned(), body: Some(body), ..Default::default() }.encode_to_vec()
    }

    fn location_event() -> GeoEvent {
        let location = FriendLocation {
            user_id: "0190d4f6-6f43-7cc2-9d6b-6f6f6f6f6f6f".into(),
            position: LatLng { lat: 35.6595, lng: 139.7005, ..Default::default() }.into(),
            ..Default::default()
        };
        GeoEvent { event: Some(Event::Location(Box::new(location))), ..Default::default() }
    }

    #[test]
    fn classification_follows_the_schema() {
        let location = frame(0, "", Body::Geo(Box::new(location_event())));
        assert_eq!(delivery_of(&location), Delivery::Droppable);
        // Envelope fields in front of the body do not confuse the scanner.
        let on_topic = frame(u64::MAX, "share:abc", Body::Geo(Box::new(location_event())));
        assert_eq!(delivery_of(&on_topic), Delivery::Droppable);

        let removed = GeoEvent { event: Some(Event::LocationRemovedUserId("someone".into())), ..Default::default() };
        assert_eq!(delivery_of(&frame(0, "", Body::Geo(Box::new(removed)))), Delivery::Reliable);
        assert_eq!(delivery_of(&frame(0, "", Body::Geo(Box::default()))), Delivery::Reliable);
        assert_eq!(delivery_of(&frame(0, "", Body::Chat(Box::default()))), Delivery::Reliable);
        assert_eq!(delivery_of(&frame(7, "live:1", topic_stats(3))), Delivery::Reliable);
    }

    #[test]
    fn garbage_is_treated_as_reliable() {
        assert_eq!(delivery_of(&[]), Delivery::Reliable);
        assert_eq!(delivery_of(&[0xff; 16]), Delivery::Reliable);
        // A geo field whose declared length runs past the end of the buffer.
        assert_eq!(delivery_of(&[0x52, 0x7f, 0x0a]), Delivery::Reliable);
        // Truncated in the middle of a varint.
        assert_eq!(delivery_of(&[0x08, 0x80]), Delivery::Reliable);
    }

    #[test]
    fn replies_echo_the_request_id() {
        let decoded = ServerFrame::decode_from_slice(&error(42, Rejection::new(Code::Unauthenticated, "TOKEN_EXPIRED"), true)).unwrap();
        assert_eq!(decoded.reply_to, 42);
        let Some(Body::Error(error)) = decoded.body else { panic!("expected an error body") };
        assert_eq!((error.code.as_str(), error.message.as_str(), error.fatal), ("unauthenticated", "TOKEN_EXPIRED", true));

        let decoded = ServerFrame::decode_from_slice(&pong(0, 99)).unwrap();
        let Some(Body::Pong(pong)) = decoded.body else { panic!("expected a pong body") };
        assert_eq!(pong.nonce, 99);
        assert!(pong.server_time.is_set());

        assert!(matches!(ServerFrame::decode_from_slice(&ack(5)).unwrap().body, Some(Body::Ack(_))));
    }
}
