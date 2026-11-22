//! LiveKit's webhook: the moment someone connects to or drops out of one
//! of our LiveKit rooms, the venue is compared with LiveKit again.
//!
//! The endpoint is an HTTP route ([`Spaces::http_routes`]) the server
//! merges into its API router. LiveKit is pointed at it in its
//! configuration:
//!
//! ```yaml
//! webhook:
//!   api_key: <the key of TOKULY_LIVEKIT_API_KEY>
//!   urls: ["https://<api host>/spaces/livekit/webhook"]
//! ```
//!
//! Requests are verified the way LiveKit signs them
//! (`tokuly_core::livekit::LiveKit::verify_webhook`): the `Authorization`
//! header is a token signed with the API secret that carries the SHA-256
//! of the body. The webhook is an accelerator, not a dependency: without
//! it the timer in [`crate::workers`] finds the same things a little later.
use std::time::Duration;

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    routing::post,
};
use connectrpc::ErrorCode;
use tokuly_core::{livekit::WebhookEvent, prelude::*};

use crate::{Spaces, model::Venue};

/// Webhooks about one venue within this time lead to one comparison.
const COALESCE: Duration = Duration::from_secs(1);

/// Path of the webhook endpoint in [`Spaces::http_routes`].
pub const WEBHOOK_PATH: &str = "/spaces/livekit/webhook";

impl Spaces {
    /// HTTP routes of this domain, to be merged into the API router: the
    /// LiveKit webhook at [`WEBHOOK_PATH`].
    pub fn http_routes(&self) -> Router {
        Router::new().route(WEBHOOK_PATH, post(receive)).with_state(self.clone())
    }

    /// Reacts to a verified webhook event: anything about who is connected
    /// to one of this domain's LiveKit rooms asks for a comparison of that
    /// venue a moment later. Events about other rooms (calls, other
    /// applications on the same LiveKit server) and other kinds of events
    /// are ignored.
    ///
    /// The comparison is not run inline: a room filling up sends an event
    /// per person, and one look a second later sees them all. Events that
    /// arrive while a look is pending are covered by it. Returns whether
    /// this event scheduled one.
    pub async fn handle_webhook(&self, event: &WebhookEvent) -> AppResult<bool> {
        let relevant = matches!(
            event.event.as_str(),
            "participant_joined"
                | "participant_left"
                | "participant_connection_aborted"
                | "room_started"
                | "room_finished"
        );
        let Some(venue) = event.room.as_ref().and_then(|room| Venue::of_room(&room.name)).filter(|_| relevant) else {
            return Ok(false);
        };
        if !self.core.cache.set_nx(&format!("spaces:compare:{}", venue.room()), "1", COALESCE).await? {
            return Ok(false);
        }
        let spaces = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            if let Err(e) = spaces.reconcile_venue(venue).await {
                tracing::warn!(?venue, error = %e, "comparison after a LiveKit webhook failed; the timer will catch up");
            }
        });
        Ok(true)
    }
}

async fn receive(State(spaces): State<Spaces>, headers: HeaderMap, body: Bytes) -> (StatusCode, &'static str) {
    let authorization = headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()).unwrap_or_default();
    let event = match spaces.livekit.verify_webhook(authorization, &body) {
        Ok(event) => event,
        Err(e) => {
            return match e.code {
                ErrorCode::Unauthenticated => (StatusCode::UNAUTHORIZED, "the webhook signature is not valid"),
                ErrorCode::InvalidArgument => (StatusCode::BAD_REQUEST, "not a webhook event"),
                _ => (StatusCode::SERVICE_UNAVAILABLE, "audio is not configured on this server"),
            };
        }
    };
    match spaces.handle_webhook(&event).await {
        Ok(_) => (StatusCode::OK, "ok"),
        Err(e) => {
            // LiveKit delivers again after a failure.
            tracing::error!(event = %event.event, error = %e, "LiveKit webhook could not be handled");
            (StatusCode::INTERNAL_SERVER_ERROR, "try again")
        }
    }
}
