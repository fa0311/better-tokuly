//! Putting a domain's event into the realtime envelope and reading it back.
//!
//! `tokuly.realtime.v1.Event` carries any domain's event as its full message
//! name plus the encoded bytes, which is what lets the realtime package (and
//! everything that only publishes or forwards frames) compile without the
//! domains' packages. These are the two ends of that.
use crate::{
    RealtimeEvent,
    realtime::{Event, ServerFrame, server_frame::Body},
};

impl Event {
    /// Wraps a domain's event message.
    pub fn pack<M: RealtimeEvent>(message: &M) -> Self {
        Self { r#type: M::FULL_NAME.to_owned(), payload: message.encode_to_vec(), ..Default::default() }
    }

    /// Whether this carries an `M`.
    pub fn is<M: RealtimeEvent>(&self) -> bool {
        self.r#type == M::FULL_NAME
    }

    /// The message inside, if it is an `M` (and decodes as one).
    pub fn unpack<M: RealtimeEvent>(&self) -> Option<M> {
        if !self.is::<M>() {
            return None;
        }
        M::decode_from_slice(&self.payload).ok()
    }
}

impl ServerFrame {
    /// The domain event this frame carries, if it is an `M`:
    /// `frame.event::<ChatEvent>()`. `None` for the gateway's own frames
    /// (welcome, pong, presence...) and for events of other types.
    pub fn event<M: RealtimeEvent>(&self) -> Option<M> {
        match &self.body {
            Some(Body::Event(event)) => event.unpack(),
            _ => None,
        }
    }
}
