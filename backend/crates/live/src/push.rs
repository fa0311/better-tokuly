//! Realtime pushes: `LiveEvent`s to a stream's topic, to single users and
//! to the channel's OBS widgets.
use tokuly_core::prelude::*;
use tokuly_proto::live::{EncoderState, LiveEvent, OverlayKind, StreamStats, live_event::Event};

use crate::{
    Live, overlays,
    streams::{self, StreamRow, topic},
};

fn wrap(event: Event) -> LiveEvent {
    LiveEvent { event: Some(event), ..Default::default() }
}

impl Live {
    /// Pushes an event to everyone following a stream.
    pub(crate) async fn push_to_stream(&self, stream_id: Uuid, event: Event) {
        self.core.rt.notify_topic(&topic(stream_id), &wrap(event)).await;
    }

    /// Pushes an event to one user's devices.
    pub(crate) async fn push_to_user(&self, user_id: Uuid, event: Event) {
        self.core.rt.notify_user(user_id, &wrap(event)).await;
    }

    /// Pushes an event to everyone following a stream but the connections
    /// of `except` (one publish; the gateways leave them out).
    pub(crate) async fn push_to_stream_except(&self, stream_id: Uuid, event: Event, except: &[Uuid]) -> AppResult<()> {
        self.core.rt.to_topic_except(&topic(stream_id), &wrap(event), except).await
    }

    /// Pushes an event to the channel's widgets of the given kinds.
    pub(crate) async fn push_to_overlays(&self, channel_id: Uuid, kinds: &[OverlayKind], event: Event) {
        match overlays::topics(self, channel_id).await {
            Ok(widgets) => {
                for (kind, topic) in widgets.iter() {
                    if kinds.iter().any(|wanted| *wanted as i32 == *kind) {
                        self.core.rt.notify_topic(topic, &wrap(event.clone())).await;
                    }
                }
            }
            Err(error) => tracing::warn!(%channel_id, %error, "could not look up the channel's widgets"),
        }
    }

    /// Tells everyone concerned that a stream changed: its topic, its
    /// owner's devices and the channel's widgets.
    pub(crate) async fn push_stream_updated(&self, stream: &StreamRow) {
        match streams::summary(&self.core, stream).await {
            Ok(summary) => {
                let event = Event::StreamUpdated(Box::new(summary));
                self.push_to_stream(stream.id, event.clone()).await;
                self.push_to_user(stream.owner_id, event.clone()).await;
                let kinds = [
                    OverlayKind::OVERLAY_KIND_CHAT_BOX,
                    OverlayKind::OVERLAY_KIND_ALERT_BOX,
                    OverlayKind::OVERLAY_KIND_GOAL,
                    OverlayKind::OVERLAY_KIND_POLL,
                ];
                self.push_to_overlays(stream.channel_id, &kinds, event).await;
            }
            Err(error) => tracing::warn!(stream_id = %stream.id, %error, "could not push a stream update"),
        }
    }

    /// Pushes the numbers that change while a stream is on air.
    pub(crate) async fn push_stats(&self, stream_id: Uuid, viewer_count: i64, peak_viewer_count: i64, like_count: i64) {
        let stats = StreamStats {
            stream_id: stream_id.to_string(),
            viewer_count,
            peak_viewer_count,
            like_count,
            ..Default::default()
        };
        self.push_to_stream(stream_id, Event::Stats(Box::new(stats))).await;
    }

    /// Tells the streamer that their encoder connected or went away.
    pub(crate) async fn push_encoder_state(&self, stream: &StreamRow, connected: bool, protocol: &str) {
        let state = EncoderState {
            stream_id: stream.id.to_string(),
            connected,
            protocol: protocol.to_owned(),
            ..Default::default()
        };
        self.push_to_user(stream.owner_id, Event::EncoderState(Box::new(state))).await;
    }
}
