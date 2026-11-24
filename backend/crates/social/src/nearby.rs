//! Nearby discovery: people with the add-friend screen open find each other.
//!
//! While a client holds the `DiscoverNearby` stream open, its user is in a
//! live GEO set (see [`crate::livegeo`]) and receives the set of other such
//! users within ~50 m whenever it changes. Being in the set is the only way
//! to see it, and users who switched off `nearby_discoverable` cannot join.
//!
//! Sessions learn about arrivals and departures without polling each other:
//! whoever joins or leaves publishes a wake-up on the bus for their map cell
//! and its neighbours, and every session listens on its own cell. A slow
//! heartbeat covers lost wake-ups and members that vanished without leaving.
//!
//! A user has one session at a time: opening a new one ends the previous
//! one (on whichever server it runs), so nobody looks from several made-up
//! positions at once.
use std::time::Duration;

use connectrpc::{ConnectError, ServiceStream};
use futures::{Stream, StreamExt as _};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokuly_core::{geoindex, prelude::*};
use tokuly_identity::settings;
use tokuly_proto::social::{DiscoverNearbyResponse, NearbyUser};

use crate::{Social, graph, livegeo::LiveGeoSet, people};

/// How often a session re-announces itself and re-reads its surroundings.
const HEARTBEAT: Duration = Duration::from_secs(4);
/// A session that misses this many heartbeats is no longer shown to others.
const PRESENCE_TTL: Duration = Duration::from_secs(15);
/// Sessions end after this long; a client still on the screen reopens.
const MAX_SESSION: Duration = Duration::from_secs(600);
/// GPS error we are willing to add to the search radius, in meters.
const MAX_ACCURACY_ALLOWANCE: f64 = 25.0;
/// Most users reported at once (a crowded venue).
const MAX_NEARBY: usize = 50;
/// Sessions one user may open per [`SESSION_WINDOW`]. The position is the
/// client's word, so every session is a look at some point of the map: few
/// of them keep it from becoming a way to scan places or close in on
/// someone.
const MAX_SESSIONS: u32 = 20;
const SESSION_WINDOW: Duration = Duration::from_secs(600);
/// What a distance is reported as: the smallest of these it does not exceed
/// (anything beyond the last is reported as the last).
const DISTANCE_BANDS: [f64; 4] = [10.0, 25.0, 50.0, 75.0];

/// A distance as shown to the other user: "within 10 / 25 / 50 / 75 m",
/// never the exact figure. Exact distances from a few self-chosen points
/// would pin anyone down to the meter.
fn coarse_distance(meters: f64) -> f64 {
    DISTANCE_BANDS.into_iter().find(|band| meters <= *band).unwrap_or(DISTANCE_BANDS[DISTANCE_BANDS.len() - 1])
}

pub(crate) const NEARBY: LiveGeoSet = LiveGeoSet::new("social:nearby:geo", "social:nearby:seen", PRESENCE_TTL);

/// Holds the ID of a user's current session; older sessions end when they
/// see it changed. Outlives the longest session a little.
fn session_key(user_id: Uuid) -> String {
    format!("social:nearby:session:{}", user_id.simple())
}
const SESSION_KEY_TTL: Duration = Duration::from_secs(MAX_SESSION.as_secs() + 60);

/// Where one session stands and how far it looks.
#[derive(Debug, Clone, Copy)]
struct Spot {
    user_id: Uuid,
    /// The user is a minor: they only meet other minors (see
    /// [`people::minors_among`]).
    minor: bool,
    lat: f64,
    lng: f64,
    radius_meters: f64,
}

/// Map cells for wake-ups: a hundredth of a degree, i.e. about a kilometer
/// (never less than ~100 m of longitude at valid latitudes), so everyone
/// within discovery range is in the same or an adjacent cell.
fn cell_of(lat: f64, lng: f64) -> (i64, i64) {
    ((lat * 100.0).floor() as i64, (lng * 100.0).floor() as i64)
}

fn wake_subject(cell: (i64, i64)) -> String {
    format!("sys.social.nearby.{}_{}", cell.0, cell.1)
}

/// The cell itself and the eight around it.
fn cell_and_neighbours(cell: (i64, i64)) -> impl Iterator<Item = (i64, i64)> {
    (-1..=1).flat_map(move |dy| (-1..=1).map(move |dx| (cell.0 + dy, cell.1 + dx)))
}

impl Social {
    /// Opens a nearby-discovery session for `user_id` at the given position.
    pub(crate) async fn discover_nearby(&self, user_id: Uuid, lat: f64, lng: f64, accuracy_meters: f64) -> AppResult<ServiceStream<DiscoverNearbyResponse>> {
        let core = &self.core;
        ensure!(geoindex::valid_coordinates(lat, lng), invalid, "INVALID_LOCATION", "a valid location is required");
        let privacy = settings::privacy_of(core, user_id).await?;
        ensure!(privacy.nearby_discoverable, precondition, "NEARBY_DISCOVERY_DISABLED", "turn on nearby discovery in your privacy settings first");
        core.cache.rate_limit(&format!("nearby:{user_id}"), MAX_SESSIONS, SESSION_WINDOW).await?;

        // This session takes over from any earlier one of the user's.
        let session = new_id().to_string();
        core.cache.set_string(&session_key(user_id), &session, Some(SESSION_KEY_TTL)).await?;

        let allowance = if accuracy_meters.is_finite() { accuracy_meters.clamp(0.0, MAX_ACCURACY_ALLOWANCE) } else { 0.0 };
        let minor = people::minors_among(core, &[user_id]).await?.contains(&user_id);
        let spot = Spot { user_id, minor, lat, lng, radius_meters: self.cfg.nearby_radius_meters + allowance };

        // Listen before showing up, so nobody arriving after our first look
        // goes unnoticed.
        let wakeups = core.bus.subscribe(wake_subject(cell_of(lat, lng))).await?;
        NEARBY.check_in(&core.cache, user_id, lat, lng).await?;
        self.announce_nearby_change(lat, lng).await;

        let (sender, receiver) = mpsc::channel(4);
        let social = self.clone();
        tokio::spawn(async move { social.run_nearby_session(spot, session, wakeups, sender).await });
        Ok(Box::pin(ReceiverStream::new(receiver).map(Ok::<_, ConnectError>)))
    }

    /// Feeds one session until the client goes away, the session times out,
    /// a newer session of the user takes over or the server shuts down, then
    /// takes the user off the map (unless a newer session keeps them there).
    async fn run_nearby_session<W, M>(&self, spot: Spot, session: String, mut wakeups: W, sender: mpsc::Sender<DiscoverNearbyResponse>)
    where
        W: Stream<Item = M> + Unpin,
    {
        let cache = &self.core.cache;
        let timeout = tokio::time::sleep(MAX_SESSION);
        tokio::pin!(timeout);
        // The first tick fires at once and produces the initial snapshot.
        let mut heartbeat = tokio::time::interval(HEARTBEAT);
        // What the client was last told: (user, relationship) in distance order.
        let mut last_sent: Option<Vec<(Uuid, i32)>> = None;
        loop {
            tokio::select! {
                _ = sender.closed() => break,
                _ = &mut timeout => break,
                _ = self.core.shutdown.cancelled() => break,
                _ = heartbeat.tick() => {
                    if let Err(e) = NEARBY.check_in(cache, spot.user_id, spot.lat, spot.lng).await {
                        tracing::warn!(error = %e, "nearby heartbeat failed");
                    }
                }
                wake = wakeups.next() => {
                    if wake.is_none() {
                        break; // the bus subscription ended; the client reconnects
                    }
                }
            }
            match cache.get_string(&session_key(spot.user_id)).await {
                Ok(current) if current.as_deref() != Some(session.as_str()) => break, // taken over
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "nearby session check failed"),
            }
            let users = match self.nearby_users(&spot).await {
                Ok(users) => users,
                Err(e) => {
                    tracing::warn!(error = %e, "nearby lookup failed; retrying on the next heartbeat");
                    continue;
                }
            };
            let seen_now: Vec<(Uuid, i32)> = users.iter().map(|(id, user)| (*id, user.relationship.to_i32())).collect();
            if last_sent.as_ref() == Some(&seen_now) {
                continue;
            }
            let response = DiscoverNearbyResponse { users: users.into_iter().map(|(_, user)| user).collect(), ..Default::default() };
            if sender.send(response).await.is_err() {
                break;
            }
            last_sent = Some(seen_now);
        }
        if let Err(e) = NEARBY.leave_unless_taken_over(cache, spot.user_id, &session_key(spot.user_id), &session).await {
            tracing::warn!(error = %e, "could not leave nearby discovery; the entry expires by itself");
        }
        self.announce_nearby_change(spot.lat, spot.lng).await;
    }

    /// Ends `user_id`'s nearby session, if one is open, and takes them off
    /// the map at once. For a user who switched `nearby_discoverable` off:
    /// the session sees its ID gone within a heartbeat and stops.
    pub(crate) async fn end_nearby_discovery(&self, user_id: Uuid) -> AppResult<()> {
        let cache = &self.core.cache;
        cache.del(&session_key(user_id)).await?;
        NEARBY.leave(cache, user_id).await
    }

    /// Wakes every session that could see a user at this position.
    async fn announce_nearby_change(&self, lat: f64, lng: f64) {
        for cell in cell_and_neighbours(cell_of(lat, lng)) {
            if let Err(e) = self.core.bus.publish(wake_subject(cell), "").await {
                tracing::warn!(error = %e, "nearby wake-up not published; sessions catch up on their heartbeat");
                return;
            }
        }
    }

    /// Everyone else in discovery around `spot`, nearest first. People in a
    /// block with the user are left out, in both directions, and so is
    /// everyone of the other age group (minors and adults).
    async fn nearby_users(&self, spot: &Spot) -> AppResult<Vec<(Uuid, NearbyUser)>> {
        let core = &self.core;
        let hidden = graph::blocked_ids_either(core, spot.user_id).await?;
        let mut found: Vec<(Uuid, f64)> = NEARBY
            .within(&core.cache, spot.lat, spot.lng, spot.radius_meters, MAX_NEARBY + 1)
            .await?
            .into_iter()
            .filter(|(id, _)| *id != spot.user_id && !hidden.contains(id))
            .collect();
        let minors = people::minors_among(core, &found.iter().map(|(id, _)| *id).collect::<Vec<_>>()).await?;
        found.retain(|(id, _)| minors.contains(id) == spot.minor);
        found.truncate(MAX_NEARBY);
        let ids: Vec<Uuid> = found.iter().map(|(id, _)| *id).collect();
        let summaries = tokuly_identity::summaries(core, &ids).await?;
        let relationships = graph::relationships(core, spot.user_id, &ids).await?;
        Ok(found
            .into_iter()
            .filter_map(|(id, distance)| {
                // Unknown or deleted accounts can linger in Valkey for a moment.
                let user = summaries.get(&id).filter(|user| !user.deleted)?.clone();
                let relationship = relationships.get(&id).copied().unwrap_or(tokuly_proto::identity::Relationship::RELATIONSHIP_NONE);
                Some((id, NearbyUser { user: user.into(), distance_meters: coarse_distance(distance), relationship: relationship.into(), ..Default::default() }))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_are_reported_in_bands() {
        assert_eq!(coarse_distance(0.0), 10.0);
        assert_eq!(coarse_distance(10.0), 10.0);
        assert_eq!(coarse_distance(10.4), 25.0);
        assert_eq!(coarse_distance(49.0), 50.0);
        assert_eq!(coarse_distance(51.0), 75.0);
        assert_eq!(coarse_distance(400.0), 75.0);
    }

    #[test]
    fn cells_are_a_hundredth_of_a_degree() {
        assert_eq!(cell_of(35.681236, 139.767125), (3568, 13976));
        assert_eq!(cell_of(-33.8568, 151.2153), (-3386, 15121));
        assert_eq!(cell_of(51.5007, -0.1246), (5150, -13));
    }

    #[test]
    fn a_change_wakes_the_cell_and_its_eight_neighbours() {
        let cells: Vec<_> = cell_and_neighbours((10, -1)).collect();
        assert_eq!(cells.len(), 9);
        assert!(cells.contains(&(10, -1)) && cells.contains(&(9, -2)) && cells.contains(&(11, 0)));
    }

    #[test]
    fn wake_subjects_are_single_nats_tokens() {
        let subject = wake_subject((-3386, 15121));
        assert_eq!(subject, "sys.social.nearby.-3386_15121");
        assert_eq!(subject.split('.').count(), 4);
    }
}
