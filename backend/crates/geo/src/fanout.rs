//! Showing people each other's pins: realtime pushes and snapshots.
//!
//! Everything here builds its messages with [`crate::visibility`]; nothing
//! else in the crate turns a [`Pin`] into a `FriendLocation`.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use tokuly_core::{prelude::*, presence};
use tokuly_proto::geo::{FriendLocation, GeoEvent, VisitEvent, geo_event::Event};
use tokuly_social::graph;

use crate::{
    Geo,
    ctx::OwnerCtx,
    geocode::Gazetteer,
    math::Point,
    state::{Area, Pin, StayPlace},
    visibility::{self, SeenPin, Sight, Subject, ViewEnv, Viewer},
};

/// Label under which the blur key is derived from the server secret.
const BLUR_KEY: &str = "geo.blur";

/// What each blurred viewer was last sent of an owner's pin is kept this long.
const SHOWN_TTL_SECONDS: i64 = 7 * 24 * 3600;

fn shown_key(owner: Uuid) -> String {
    format!("geo:pin:shown:{}", owner.simple())
}

/// How long someone who loaded friends' pins (`ListFriendLocations`, a
/// `WatchFriendLocations` stream) counts as watching without a refresh (the
/// stream refreshes at a third of this). Longer than the gateway takes to
/// mark an app that came back as open.
pub(crate) const WATCHING_TTL: std::time::Duration = std::time::Duration::from_secs(90);

fn watching_key(viewer: Uuid) -> String {
    format!("geo:watching:{}", viewer.simple())
}

fn area_lookup(lat: f64, lng: f64) -> Area {
    Gazetteer::shared().area(lat, lng)
}

static AREA_LOOKUP: fn(f64, f64) -> Area = area_lookup;

pub(crate) fn frame(event: Event) -> GeoEvent {
    GeoEvent { event: Some(event), ..Default::default() }
}

/// Who a pin update goes to.
pub(crate) enum Recipients<'a> {
    /// Every friend of the owner.
    Friends,
    /// These users (the caller made sure they are friends).
    Only(&'a [Uuid]),
}

/// Something that happened at a place, to tell friends about.
#[derive(Debug, Clone)]
pub(crate) struct VisitNews {
    pub arrived: bool,
    pub at: DateTime<Utc>,
    pub place: Option<StayPlace>,
}

impl Geo {
    /// The surroundings the visibility functions need.
    pub(crate) fn view_env(&self, now: DateTime<Utc>) -> ViewEnv<'_> {
        ViewEnv { now, mac: &*self.blur_mac, area_at: &AREA_LOOKUP }
    }

    pub(crate) fn make_blur_mac(core: &Core) -> Box<dyn Fn(&[u8]) -> [u8; 32] + Send + Sync> {
        let core = core.clone();
        Box::new(move |data| core.keys.mac(BLUR_KEY, data))
    }

    /// Marks `viewer` as watching friends' pins: they loaded the snapshot
    /// and receive pushes from now on, as an open app does.
    pub(crate) async fn mark_watching(&self, viewer: Uuid) -> AppResult<()> {
        self.core.cache.set_string(&watching_key(viewer), "1", Some(WATCHING_TTL)).await
    }

    /// Those of `friends` who see pins live: the app open, or the pins
    /// loaded a moment ago (a `WatchFriendLocations` stream keeps saying so).
    async fn watching(&self, friends: &[Uuid]) -> AppResult<HashSet<Uuid>> {
        let mut watching: HashSet<Uuid> = presence::online_among(&self.core.cache, friends).await?.into_iter().collect();
        let keys: Vec<String> = friends.iter().map(|id| watching_key(*id)).collect();
        let streams: Vec<Option<String>> = redis::cmd("MGET").arg(&keys).query_async(&mut self.core.cache.conn()).await?;
        watching.extend(friends.iter().zip(streams).filter(|(_, v)| v.is_some()).map(|(id, _)| *id));
        Ok(watching)
    }

    /// Pushes a geo event to every connected device of one user.
    pub(crate) async fn push(&self, user_id: Uuid, event: Event) {
        self.core.rt.notify_user(user_id, &frame(event)).await;
    }

    /// Pushes the same geo event to several users.
    pub(crate) async fn push_many(&self, user_ids: impl IntoIterator<Item = Uuid>, event: Event) {
        self.core.rt.notify_users(user_ids, &frame(event)).await;
    }

    /// Whether each user's account language is Japanese (cached for a few minutes).
    pub(crate) async fn languages(&self, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, bool>> {
        let mut out = HashMap::with_capacity(user_ids.len());
        let mut unknown = Vec::new();
        for user_id in user_ids {
            match self.locales.get(user_id) {
                Some(ja) => {
                    out.insert(*user_id, ja);
                }
                None => unknown.push(*user_id),
            }
        }
        if !unknown.is_empty() {
            let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, locale FROM users WHERE id = ANY($1)")
                .bind(&unknown)
                .fetch_all(&self.core.db)
                .await?;
            for (user_id, locale) in rows {
                let ja = locale.to_ascii_lowercase().starts_with("ja");
                self.locales.insert(user_id, ja);
                out.insert(user_id, ja);
            }
        }
        Ok(out)
    }

    pub(crate) async fn speaks_japanese(&self, user_id: Uuid) -> AppResult<bool> {
        // Japanese first: an account we know nothing about gets Japanese names.
        Ok(self.languages(&[user_id]).await?.get(&user_id).copied().unwrap_or(true))
    }

    /// Forgets a cached account language (the account changed or was deleted).
    pub(crate) fn forget_language(&self, user_id: Uuid) {
        self.locales.invalidate(&user_id);
    }

    /// Pushes the owner's pin to friends, each seeing it through their own
    /// ghost rule. Friends who see a frozen pin get nothing — a frozen pin
    /// does not move — unless `refresh` is set, which re-sends everyone their
    /// current view (after a change to ghost rules, privacy or status).
    ///
    /// Only friends who watch (the app open, or the pins just loaded) get a
    /// push: the others load every pin when they open it (`ListFriendLocations`).
    pub(crate) async fn fan_out(
        &self,
        owner: Uuid,
        pin: Option<&Pin>,
        ctx: &OwnerCtx,
        recipients: Recipients<'_>,
        refresh: bool,
    ) -> AppResult<()> {
        let friends = match recipients {
            Recipients::Friends => graph::friend_ids(&self.core, owner).await?,
            Recipients::Only(ids) => ids.to_vec(),
        };
        if friends.is_empty() || !ctx.active {
            return Ok(());
        }
        let watching = self.watching(&friends).await?;
        let online = presence::is_online(&self.core.cache, owner).await.unwrap_or(false);
        let weather = self.weather_at(&[pin.map(Pin::position)]).await.pop().flatten();
        let languages = self.languages(&friends).await?;
        let now = Utc::now();
        let env = self.view_env(now);
        let subject = Subject { id: owner, pin, ctx, online, weather: weather.as_ref() };
        // What viewers who do not see the precise pin were last sent: they
        // get a push when what THEY see changes, not whenever the owner
        // moves — how often an unchanged stand-in arrives would say whether
        // its owner is walking or sitting still.
        let key = shown_key(owner);
        let any_blurred = friends.iter().any(|viewer| matches!(visibility::sight(ctx, *viewer, now), Sight::Blurred));
        let mut shown: HashMap<String, String> = if any_blurred {
            redis::cmd("HGETALL").arg(&key).query_async(&mut self.core.cache.conn()).await?
        } else {
            HashMap::new()
        };
        let mut changed = Vec::new();
        for viewer in friends {
            let sight = visibility::sight(ctx, viewer, now);
            if !refresh && matches!(sight, Sight::Frozen(_)) {
                continue;
            }
            let watches = watching.contains(&viewer);
            if !watches && !matches!(sight, Sight::Blurred) {
                continue;
            }
            let viewer = Viewer { id: viewer, ja: languages.get(&viewer).copied().unwrap_or(true) };
            let Some(view) = visibility::friend_view(&env, &subject, viewer) else { continue };
            if matches!(sight, Sight::Blurred) {
                let field = viewer.id.simple().to_string();
                let now_shown = serde_json::to_string(&view).unwrap_or_default();
                if shown.get(&field) == Some(&now_shown) && !refresh {
                    continue;
                }
                // Kept for those who are away too: back with the snapshot,
                // their next push must still wait for a change they can see.
                shown.insert(field.clone(), now_shown.clone());
                changed.push((field, now_shown));
            }
            if watches {
                self.push(viewer.id, Event::Location(Box::new(view))).await;
            }
        }
        if !changed.is_empty() {
            let mut conn = self.core.cache.conn();
            redis::cmd("HSET").arg(&key).arg(&changed).query_async::<()>(&mut conn).await?;
            redis::cmd("EXPIRE").arg(&key).arg(SHOWN_TTL_SECONDS).query_async::<()>(&mut conn).await?;
        }
        Ok(())
    }

    /// Re-sends every friend their current view of the owner's pin.
    pub(crate) async fn refresh_friends(&self, owner: Uuid) -> AppResult<()> {
        let ctx = self.ctx_of(owner).await?;
        let state = self.load_state(owner).await?;
        let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
        self.fan_out(owner, pin, &ctx, Recipients::Friends, true).await
    }

    /// Shows friends the owner's pin again after something that comes from
    /// their whereabouts changed on it (a trip, the place they are at).
    /// Friends who see a frozen pin get nothing: for them nothing changed,
    /// and a push would tell them that something did.
    pub(crate) async fn refresh_friends_live(&self, owner: Uuid) -> AppResult<()> {
        let ctx = self.ctx_of(owner).await?;
        let state = self.load_state(owner).await?;
        let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
        self.fan_out(owner, pin, &ctx, Recipients::Friends, false).await
    }

    /// Re-sends some friends their current view of the owner's pin.
    pub(crate) async fn refresh_viewers(&self, owner: Uuid, viewers: &[Uuid]) -> AppResult<()> {
        if viewers.is_empty() {
            return Ok(());
        }
        let ctx = self.ctx_of(owner).await?;
        let state = self.load_state(owner).await?;
        let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
        self.fan_out(owner, pin, &ctx, Recipients::Only(viewers), true).await
    }

    /// Publishes the owner's pin on the topic of each of their live shares.
    pub(crate) async fn publish_shares(&self, owner: Uuid, pin: &Pin, ctx: &OwnerCtx) {
        let now = Utc::now();
        let live: Vec<&str> = ctx
            .shares
            .iter()
            .filter(|share| share.expires_ms > now.timestamp_millis())
            .map(|share| share.token.as_str())
            .collect();
        if live.is_empty() {
            return;
        }
        let subject = Subject { id: owner, pin: Some(pin), ctx, online: false, weather: None };
        let Some(view) = visibility::share_view(&self.view_env(now), &subject, ctx.ja) else { return };
        for token in live {
            self.core
                .rt
                .notify_topic(&crate::shares::topic(token), &frame(Event::Location(Box::new(view.clone()))))
                .await;
        }
    }

    /// Tells friends that the owner arrived at or left a place — those who
    /// see the precise pin, and only what each may know about the place.
    pub(crate) async fn push_visit_news(&self, owner: Uuid, ctx: &OwnerCtx, news: &[VisitNews]) -> AppResult<()> {
        let with_place: Vec<(&VisitNews, &StayPlace)> =
            news.iter().filter_map(|item| item.place.as_ref().map(|place| (item, place))).collect();
        if with_place.is_empty() || !ctx.active {
            return Ok(());
        }
        let now = Utc::now();
        for viewer in graph::friend_ids(&self.core, owner).await? {
            if !visibility::sees_precisely(ctx, viewer, now) {
                continue;
            }
            let share_labels = ctx.privacy.share_place_labels || ctx.sos_for(viewer);
            for (item, place) in &with_place {
                let Some(shown) = visibility::shown_place(owner, place, share_labels, Some(viewer), true) else {
                    continue;
                };
                let event = VisitEvent {
                    user_id: owner.to_string(),
                    arrived: item.arrived,
                    place_label: shown.label.into(),
                    place_name: shown.name,
                    at: ts(item.at),
                    ..Default::default()
                };
                self.push(viewer, Event::Visit(Box::new(event))).await;
            }
        }
        Ok(())
    }

    /// Every friend's pin as `viewer` may see it.
    pub(crate) async fn friend_locations(&self, viewer: Uuid) -> AppResult<Vec<FriendLocation>> {
        let friends = graph::friend_ids(&self.core, viewer).await?;
        if friends.is_empty() {
            return Ok(Vec::new());
        }
        let states = self.load_states(&friends).await?;
        let contexts = self.ctx_many(&friends).await?;
        let online: HashSet<Uuid> = presence::online_among(&self.core.cache, &friends).await?.into_iter().collect();
        let positions: Vec<Option<Point>> =
            states.iter().map(|state| state.as_ref().filter(|s| s.located()).map(|s| s.pin.position())).collect();
        let weather = self.weather_at(&positions).await;
        let now = Utc::now();
        let env = self.view_env(now);
        let viewer = Viewer { id: viewer, ja: self.speaks_japanese(viewer).await? };
        let mut out = Vec::new();
        for ((friend, state), weather) in friends.iter().zip(&states).zip(&weather) {
            let Some(ctx) = contexts.get(friend) else { continue };
            let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
            let subject = Subject { id: *friend, pin, ctx, online: online.contains(friend), weather: weather.as_ref() };
            if let Some(view) = visibility::friend_view(&env, &subject, viewer) {
                out.push(view);
            }
        }
        Ok(out)
    }

    /// One friend's pin as `viewer` may see it. The caller checked the friendship.
    pub(crate) async fn location_as_seen(&self, owner: Uuid, viewer: Uuid) -> AppResult<Option<FriendLocation>> {
        let ctx = self.ctx_of(owner).await?;
        let state = self.load_state(owner).await?;
        let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
        let online = presence::is_online(&self.core.cache, owner).await.unwrap_or(false);
        let weather = self.weather_at(&[pin.map(Pin::position)]).await.pop().flatten();
        let now = Utc::now();
        let subject = Subject { id: owner, pin, ctx: &ctx, online, weather: weather.as_ref() };
        let viewer = Viewer { id: viewer, ja: self.speaks_japanese(viewer).await? };
        Ok(visibility::friend_view(&self.view_env(now), &subject, viewer))
    }

    /// The owner's own pin, with nothing withheld.
    pub(crate) async fn own_location(&self, owner: Uuid) -> AppResult<(Option<FriendLocation>, bool)> {
        let ctx = self.ctx_of(owner).await?;
        let state = self.load_state(owner).await?;
        let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
        let online = presence::is_online(&self.core.cache, owner).await.unwrap_or(false);
        let weather = self.weather_at(&[pin.map(Pin::position)]).await.pop().flatten();
        let subject = Subject { id: owner, pin, ctx: &ctx, online, weather: weather.as_ref() };
        let sharing = pin.is_some_and(|pin| pin.sharing);
        Ok((visibility::own_view(&self.view_env(Utc::now()), &subject), sharing))
    }

    /// Where `viewer` has their friend `owner` on the map (see
    /// [`visibility::seen_pin`]).
    pub(crate) async fn seen_pin(&self, owner: Uuid, viewer: Uuid) -> AppResult<Option<SeenPin>> {
        let ctx = self.ctx_of(owner).await?;
        let state = self.load_state(owner).await?;
        let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
        let subject = Subject { id: owner, pin, ctx: &ctx, online: false, weather: None };
        Ok(visibility::seen_pin(&self.view_env(Utc::now()), &subject, viewer))
    }

    /// The friends of `owner` who see their precise, live position right
    /// now: the only people who may be told anything derived from it.
    pub(crate) async fn precise_viewers(&self, owner: Uuid, ctx: &OwnerCtx) -> AppResult<Vec<Uuid>> {
        let audience = visibility::Audience::of(ctx, Utc::now());
        Ok(graph::friend_ids(&self.core, owner)
            .await?
            .into_iter()
            .filter(|friend| audience.includes(*friend))
            .collect())
    }

    /// The friends of `viewer` whose precise, live location `viewer` sees:
    /// the only people whose whereabouts (past visits, gatherings) may be
    /// shown to them.
    pub(crate) async fn precise_friends(&self, viewer: Uuid) -> AppResult<Vec<Uuid>> {
        let friends = graph::friend_ids(&self.core, viewer).await?;
        let contexts = self.ctx_many(&friends).await?;
        let now = Utc::now();
        Ok(friends
            .into_iter()
            .filter(|friend| contexts.get(friend).is_some_and(|ctx| visibility::sees_precisely(ctx, viewer, now)))
            .collect())
    }
}
