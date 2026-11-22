# geo — events, jobs and configuration

Crate `tokuly-geo`, migration `0300_geo.sql`, ClickHouse table
`backend/clickhouse/0300_geo.sql` (`geo_points`), API `tokuly.geo.v1`
(`LocationService`, `PlaceService`, `HistoryService`, `FootprintService`,
`TripService`, `SafetyService`, realtime `GeoEvent`).

## How a location sample travels

```
device ──▶ realtime gateway ──▶ LOCATIONS stream (loc.sample.<user>) ──┐
device ──▶ LocationService.ReportLocations ────────────────────────────┤
simulator ──▶ Geo::ingest ─────────────────────────────────────────────┤
                                                                       ▼
                                      pipeline: per user, in order, under a Valkey lock
   ┌───────────────┬───────────────────┬──────────────┬───────────────┬───────────────────┐
 live state      stays → visits,     footprints     raw history     live samples only:   pin to each friend
 (Valkey) and    places, nights,     (PostGIS:      (ClickHouse     place alerts, trip    through their ghost
 core geoindex   hourly distance     cells+regions)  geo_points)    ETA, gatherings       rule; live shares
```

Samples that are unusable (no or impossible coordinates, accuracy negative or
worse than 10 km, recorded more than 2 minutes in the future or longer ago
than `TOKULY_GEO_MAX_SAMPLE_AGE_HOURS`), duplicates and samples older than the
newest one already processed are skipped. Fixes worse than 100 m move the pin
only when no good fix arrived in the last three minutes and never feed stays,
distance or footprints. Mock locations move the pin but leave no footprints.
A user who switched off `record_history` still shares live (pin, time on the
spot, place badge) but gets no visits, nights, distance, footprints or raw
points.

## What a viewer sees (ghost mode)

One function decides (`tokuly_geo::visibility`), for `ListFriendLocations`,
`WatchFriendLocations`, every `GeoEvent.location` push, trips towards a
friend, profile distances, gatherings, place cards and place alerts:

| Mode | The viewer gets |
|---|---|
| PRECISE | the true pin, minus what `share_battery`, `share_activity`, `share_online_status`, `share_place_labels` withhold |
| BLURRED | a point 300 m–1.2 km away that depends only on (owner, viewer, the surrounding ~5 km² cell): still during a stay, the same on every visit, different per viewer; no speed, heading, activity, time on the spot, place or trip; area name and time zone of the blurred point |
| FROZEN | the pin as it was when freezing began, with its original `recorded_at` and battery — the same message a phone that stopped reporting would produce; only the online halo and status stay live; nothing is pushed |
| SOS active | the people alerted get the precise live pin with nothing withheld, whatever the rules say |

Rules are a default for everyone plus per-friend overrides, optionally timed.
An override saying PRECISE is kept as an exception while the default is not
PRECISE. Home, work and school are a badge for friends, never a name.

## Events emitted

| Topic | Payload | When |
|---|---|---|
| `notify.requested` | `NotificationRequest` | see "Notification kinds" |
| `activity.recorded` | `ActivityEntry` | see "Activity entries" |
| `search.changed` | `SearchChange` (kind `place`) | a place became public, changed while public, stopped being public or was deleted |

The geo domain emits no domain events of its own: what other domains need
from it is live (the `tokuly_core::geoindex` position index) or goes through
the three generic topics.

## Events consumed

Durable consumer `geo` (`Geo::handle_event`). Every handler is idempotent.

| Topic | Reaction |
|---|---|
| `identity.user.deleted` | Deletes every row about the user (places with their photos' holds and search documents, saved places, visits, nights, hourly totals, memos and their holds, cells, region stats, trips from/to them, alerts by/about them, SOS, live shares, gathering membership, bumps, ghost rules by/for them), their raw points in ClickHouse, live state, cached context and geoindex entry; ends their live shares on the topic; takes them off other people's active SOS. |
| `identity.user.privacy_changed` | Rebuilds the cached context, mirrors `footprints_visible_to` into `geo_users`, closes the open visit and drops pending distance when `record_history` was switched off, and re-pushes the pin to friends (battery/activity/halo/badge follow at once). |
| `identity.user.status_changed`, `identity.user.updated` | Rebuilds the cached context (status emoji, language) and re-pushes the pin to friends. |
| `identity.user.banned` / `identity.user.unbanned` | Banned: `location_removed_user_id` to friends, off the geoindex, out of gatherings, samples ignored. Unbanned: pin pushed again. |
| `social.friend.accepted` | Each of the two gets the other's pin at once. |
| `social.friend.removed`, `social.user.blocked` | `location_removed_user_id` to both; drops ghost overrides, place alerts and trips between the two, takes each off the other's trip share lists and active SOS, un-saves each other's non-public places. |
| `social.bump.created` | Records the bump (`geo_bumps`, for `GetStats.bump_count`) and calls `tokuly_social::bumps::set_bump_place` with a place both may know (a public place, or one of theirs shared with friends) or else the area name (Japanese first). |
| `admin.content.removed` | `SUBJECT_TYPE_PLACE`: deletes the place (photo hold, search document, saved entries; visits keep the name). |
| `media.asset.deleted` | Removes the reference from the place photo / day memo that used the asset. |
| `search.reindex.requested` | `kind = "place"`: publishes the search document of every public place again (200 per transaction, under a row lock so a place that stops being public meanwhile is unpublished afterwards). Other kinds are ignored. |

## Bus subjects

| Subject | Direction | Purpose |
|---|---|---|
| `loc.sample.<user>` (JetStream `LOCATIONS`) | consumed, durable `geo-pipeline`, shared by worker processes | `tokuly_core::presence::LocationReport` from the realtime gateway. Spread over `TOKULY_GEO_PIPELINE_LANES` lanes by user so one user's samples stay in order; retried up to 5 times with backoff, undecodable payloads are dropped. |
| `rt.u.<user>` | published (and read by `WatchFriendLocations` for its own caller) | `GeoEvent` frames to users. |
| `rt.t.share:<token>` | published (and read by `WatchLiveShare`) | Live share topic, see below. |

## Realtime pushes

`ServerFrame.geo` (`GeoEvent`), user-scoped unless noted:

| Event | Sent to | When |
|---|---|---|
| `location` | each friend, in their own view | An accepted sample changed the pin (moved 5 m or more, or battery/activity/place/stay/pause changed) or 30 s passed; and at once after a change to ghost rules, privacy, status, a trip, an SOS, or a new friendship. Friends under FROZEN are skipped. |
| `location` on topic `share:<token>` | anyone subscribed | The sharer's pin (public view: position and movement, no badge, halo or status). |
| `location_removed_user_id` | both sides / friends | Unfriended, blocked, banned. On `share:<token>`: the share ended (revoked, expired, account deleted). |
| `visit` | friends who see the precise pin | Arrival at / departure from a place they may know about (badge for home/work/school, name for shared or public places), and check-ins. |
| `gathering_updated` / `gathering_ended_id` | members and their friends | Membership changed. Each viewer gets only members who are their friends and visible precisely to them; with fewer than two such members they get `gathering_ended_id`. |
| `trip_updated` | the traveller and the people the trip is shared with | Started, progress (at most every 20 s unless the ETA moved by a minute), arrived, cancelled, replaced. Destination coordinates and route only where the viewer may have them. |
| `place_alert_updated` | owner and subject | Created, answered, lapsed, removed (as status EXPIRED), fired once. |
| `place_alert_fired` | owner | The alert went off. |
| `sos_updated` | the sender and the people alerted | Triggered, resolved, auto-resolved. |
| `footprint_progress` | the user themselves | New cells; one extra event per region entered for the first time. |

Topic `share:<token>` is deliberately unguarded: the 32-character token is the
capability.

## Notification kinds

All through `tokuly_core::outbound::notify`; the actor is the other person.

| Kind | Recipient | Notes |
|---|---|---|
| `geo.gathering` | friends of at least two members who see both precisely | "AさんとBさんが集合中!" — when a gathering forms, not at a member's home/work/school, at most once per pair every 3 hours, to at most 100 people. `data = { gathering_id, user_ids }`. |
| `geo.checkin_tagged` | friends tagged in a check-in | |
| `geo.checkin` | other friends who see the precise pin (at most 200) | Collapsed per user. Not sent under ghost mode. |
| `geo.new_region` | the user themselves (no actor) | First time in a country or first-level subdivision, except the ones the account starts in. Collapsed per region. |
| `geo.trip_started` | friends a trip was explicitly shared with | Not sent for trips shared with all friends. |
| `geo.trip_arrived` | the trip's audience | Only when `notify_on_arrival` was set. |
| `geo.place_alert_set` | subject | A one-time alert about them is active. |
| `geo.place_alert_request` | subject | A recurring alert awaits their approval. |
| `geo.place_alert_response` | owner | Approved or declined; `data = { alert_id, approved }`. |
| `geo.place_alert` | owner | The alert fired; `data = { alert_id, arrived }`. Collapsed per alert. |
| `geo.sos` | the people alerted | **Urgent** (`.urgent()`). Collapsed per SOS. |
| `geo.sos_resolved` | the people alerted | Resolved by the sender or after `TOKULY_GEO_SOS_MAX_HOURS`. |

## Activity entries

| Kind | Audience | Notes |
|---|---|---|
| `geo.visit` | friends, with location — private and without location while the default ghost mode is not PRECISE | A check-in (visits found by stay detection are never published). `with_user_ids` = tagged friends; subject = the place unless it is private; `data = { visit_id, message, place_name, check_in: true }` (`place_name` in the user's language: the place, or the area when the place is private) |
| `geo.new_region` | friends — private for a user whose footprints are visible to nobody | location = region center; `data = { region_id, region_code, region_name, kind, country_code }` (`region_code` = the region ID such as `JP-14`; `region_name` in the user's language) |

`place_name`, `region_code` and `region_name` are the fields the feed domain's recaps read (`docs/events/feed.md`).

Order across worker processes: within one process a user's samples are
processed in the order they were published. Several worker processes share
the durable consumer, so two samples of one user can reach different
processes and finish in the other order; the state only moves forward, so the
older one is then dropped (it is missing from the raw history, nothing else).
Devices report every few seconds at most, which makes this rare; a deployment
that wants none of it runs the location pipeline in one worker process.

## Background work

No job-queue jobs. `Geo::spawn_workers` starts:

| Worker | Schedule | What it does |
|---|---|---|
| location pipeline | on every `loc.sample.*` message | See above. |
| event consumer | on events | See "Events consumed". |
| history flusher (`Geo::flush_history`) | every 2 s, and at shutdown | Writes buffered raw points to ClickHouse. `ReportLocations` also flushes before it returns; a ClickHouse outage keeps up to 200,000 rows buffered and never fails location sharing. |
| sweeper (`Geo::sweep`) | every 15 s | Ends timed ghost rules (viewers get the live pin), live shares past their expiry, place alerts that waited too long, SOS older than the maximum, trips abandoned for 6 h or running for 24 h, and gathering members silent for 45 minutes. |
| daily pass (`Geo::enforce_retention`, `Geo::prune`) | 5 minutes after start, then every 24 h | Deletes raw points older than each user's `location_history_days` (0 = keep forever); deletes ended alerts (30 days), ended live shares (7 days), and gatherings, SOS and trips over for a year. |

## Valkey keys

| Key | Type, TTL | Purpose |
|---|---|---|
| `geo:state:<user>` | JSON, 60 days | Live state: latest pin, stay tracker, pending distance, proximity tracker, recent cells. |
| `geo:ctx:<user>` | JSON, 5 min | Cached projection of the user's privacy flags, status, ghost rules, own places, active SOS/trip/shares and the alerts watching them. Dropped on every change. |
| `geo:ctx:dirty:<user>` | marker, 5 s | Keeps a reader that raced a writer from re-caching stale context. |
| `geo:lock:<user>` | token, 30 s | Serializes the pipeline and state-editing RPCs per user. |
| `geo:pos`, `geo:pos:ts` | `tokuly_core::geoindex` | True latest positions for other domains (written here on every accepted sample, removed on StopSharing / ban / deletion). |
| `geo:wx:<lat>:<lng>`, `geo:wx:fetch:…` | JSON 30 min / marker 5 min | Weather per 0.1° cell and the "already fetching" marker. |
| `geo:gathering:announced:<lo>:<hi>` | marker, 3 h | Throttles "A and B are together". |
| `geo:share:seen:<share>:<ip>` | marker, 1 h | Counts each visitor of a share link once. |

Rate limits (`rl:` keys): uploads 240/min; watch streams 30/min; ghost changes
60/min; place creation 60/h; place search 120/min; reverse geocoding 240/min;
check-ins 30/h; live shares 20/h, link lookups 120/min and streams 30/min per
IP; trips 30/h; directions 60/min; place alerts 30/h; SOS 10/h; history
deletion 20/h.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_GEOCODER_URL` | empty | Photon base URL for `SearchPlaces` and `ReverseGeocode`. Empty or failing: the bundled gazetteer. |
| `TOKULY_ROUTING_URL` | empty | Valhalla base URL for directions and trip plans. Empty or failing: great-circle estimate (`provider = "estimate"`). |
| `TOKULY_WEATHER_URL` | empty | Open-Meteo compatible base URL. Empty: no weather on pins. |
| `TOKULY_GEO_STAY_RADIUS_METERS` / `TOKULY_GEO_STAY_SECONDS` | `80` / `300` | A stay: within this radius for this long. |
| `TOKULY_GEO_GATHERING_RADIUS_METERS` / `TOKULY_GEO_GATHERING_SECONDS` | `30` / `120` | A gathering: friends within this radius for this long. |
| `TOKULY_GEO_MOVING_INTERVAL_SECONDS` / `TOKULY_GEO_STATIONARY_INTERVAL_SECONDS` | `5` / `300` | Upload cadence suggested to clients. |
| `TOKULY_GEO_MAX_SAMPLE_AGE_HOURS` | `168` | Oldest sample accepted from a backlog upload. |
| `TOKULY_GEO_HOME_NIGHTS` | `3` | Nights at one spot before it is suggested as home. |
| `TOKULY_GEO_ONE_TIME_ALERT_HOURS` / `TOKULY_GEO_PENDING_ALERT_DAYS` | `24` / `7` | How long a one-time alert waits to fire / a recurring one waits for approval. |
| `TOKULY_GEO_SOS_MAX_HOURS` | `24` | An unresolved SOS ends by itself after this long. |
| `TOKULY_GEO_MAX_CELLS` | `20000` | Cells per `GetCells` response. |
| `TOKULY_GEO_PIPELINE_LANES` | `16` | Parallel lanes of the location consumer per worker process. |

Infrastructure assumptions: PostGIS (already created by `0001_core.sql`) for
regions and place search; ClickHouse for raw history with lightweight
`DELETE` available (default in current versions).

## Public Rust API

```rust
impl Geo {
    pub fn new(core: Core, identity: Identity, cfg: GeoConfig) -> anyhow::Result<Self>;
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router;
    pub fn spawn_workers(&self);
    /// Loads the bundled boundaries into geo_regions. Idempotent; call at startup after migrations.
    pub async fn seed_regions(&self) -> AppResult<()>;
    /// Processes one report directly, as the stream consumer would (simulators, tools).
    pub async fn ingest(&self, report: tokuly_core::presence::LocationReport) -> AppResult<()>;
    pub async fn handle_event(&self, event: &tokuly_core::events::Event) -> AppResult<()>;
    pub async fn flush_history(&self) -> AppResult<()>;
    pub async fn sweep(&self);
    pub async fn enforce_retention(&self) -> AppResult<u64>;
    pub async fn prune(&self) -> AppResult<()>;
    pub fn reload_regions(&self);
}
pub const CLICKHOUSE_SCHEMA: &[(&str, &str)];   // for analytics::apply_schema in the server's setup step
```

`ingest` buffers raw history like the stream consumer does: a process that
does not run `spawn_workers` (a simulator) calls `flush_history` when done.

`Geo::new` registers a `tokuly_identity::ProfileExtension`
(`Profile.distance_meters` for friends — the true distance when the viewer
sees the precise pin, the distance to the blurred point when blurred, unset
when frozen, paused or unknown — plus `stats.explored_percent` of the home
country and `stats.visited_country_count`), a `SubjectResolver` for
`SUBJECT_TYPE_PLACE` (path `/places/<id>`), and a `DataExporter` named `geo`.

Other domains read positions through `tokuly_core::geoindex` (true positions,
for rule checks only) — never through this crate, and never to show a
location to a user: only the functions above apply ghost mode.

## For the server crate

```rust
// DomainConfigs
#[command(flatten)]
pub geo: GeoConfig,

// Domains::new: after identity, before Engagement (so the PLACE resolver is
// registered when engagement looks). Every role: the profile extension, the
// subject resolver and the exporter live in this process's registries.
let geo = Geo::new(core.clone(), identity.clone(), cfg.geo)?;

// connect_router
router = self.geo.register(router);

// migrate, after the database migrations (needs PostGIS and 0300_geo.sql):
domains.geo.seed_regions().await?;   // no-op when the bundled version is already stored; a few seconds the first time
// ...and tokuly_geo::CLICKHOUSE_SCHEMA (the `geo_points` table) appended to
// the list given to analytics::apply_schema.

// spawn_workers (worker role)
domains.geo.spawn_workers();
```

No bridge to another domain is needed: geo depends on core, proto, identity,
social and media only, and other domains reach it through `core.subjects`,
the outbox (`search.changed`, notifications, activity entries) and
`tokuly_core::geoindex`.

Without ClickHouse everything but the raw history works: `GetDay` returns
the day without its path, `GetRoute` and `DeleteHistory` fail with
`HISTORY_UNAVAILABLE` (nothing is deleted), and points wait in the buffer
(see the flusher).
Without `seed_regions` footprints still count cells, with no regions and no
percentages.

## Schema notes

- `geo_regions` holds the bundled boundaries (Natural Earth, simplified; see
  `backend/crates/geo/data/README.md`). Region IDs are ISO codes (`JP`,
  `JP-13`); `world` is a pseudo-region. Percent explored = summed area of the
  user's resolution-9 cells whose center lies in the region / region area.
- `geo_cells` keeps resolution 9 (the truth, with the country and subdivision
  of each cell) and materialized parents at 7 and 5 for low zoom levels.
  `first_at` is the device time, `uncovered_at` the time the server learned of
  the cell (what the recap goes by).
- `geo_activity_hours` holds distance and time per user, UTC hour and
  activity; days are cut in whatever time zone a request asks for.
  `DeleteHistory` removes whole hour buckets that overlap the range.
- Reverse geocoding uses the bundled GeoNames gazetteer (attribution required:
  "Place names © GeoNames, CC BY 4.0"); time zones come from `tzf-rs`.
