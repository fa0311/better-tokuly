//! Helpers shared by the identity test binaries.
#![allow(dead_code)]
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
};

use async_trait::async_trait;
use axum::body::Body;
use http_body_util::BodyExt as _;
use tokuly_core::prelude::*;
use tokuly_identity::{Identity, IdentityConfig, ProfileExtension};
use tokuly_proto::identity::{Profile, Relationship};
use tokuly_testkit::TestApp;
use tower::ServiceExt as _;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0400"];

/// Friendships and blocks, standing in for the social domain.
#[derive(Default)]
pub struct FakeSocial {
    friends: Mutex<HashSet<(Uuid, Uuid)>>,
    blocks: Mutex<HashSet<(Uuid, Uuid)>>,
    /// (follower, followee)
    follows: Mutex<HashSet<(Uuid, Uuid)>>,
    locked: Mutex<HashSet<Uuid>>,
    /// (owner, member)
    close_friends: Mutex<HashSet<(Uuid, Uuid)>>,
    /// (circle, member)
    circles: Mutex<HashSet<(Uuid, Uuid)>>,
}

fn pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a < b { (a, b) } else { (b, a) }
}

impl FakeSocial {
    pub fn befriend(&self, a: Uuid, b: Uuid) {
        self.friends.lock().unwrap().insert(pair(a, b));
    }

    pub fn block(&self, blocker: Uuid, blocked: Uuid) {
        self.blocks.lock().unwrap().insert((blocker, blocked));
    }

    pub fn follow(&self, follower: Uuid, followee: Uuid) {
        self.follows.lock().unwrap().insert((follower, followee));
    }

    pub fn lock(&self, owner: Uuid, locked: bool) {
        let mut set = self.locked.lock().unwrap();
        if locked {
            set.insert(owner);
        } else {
            set.remove(&owner);
        }
    }

    pub fn close_friend(&self, owner: Uuid, member: Uuid) {
        self.close_friends.lock().unwrap().insert((owner, member));
    }

    pub fn join_circle(&self, circle: Uuid, member: Uuid) {
        self.circles.lock().unwrap().insert((circle, member));
    }

    fn blocked_either(&self, a: Uuid, b: Uuid) -> bool {
        let blocks = self.blocks.lock().unwrap();
        blocks.contains(&(a, b)) || blocks.contains(&(b, a))
    }

    fn are_friends(&self, a: Uuid, b: Uuid) -> bool {
        self.friends.lock().unwrap().contains(&pair(a, b))
    }
}

#[async_trait]
impl ProfileExtension for FakeSocial {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        if let Some(viewer) = viewer
            && viewer != owner
            && self.are_friends(viewer, owner)
        {
            profile.relationship = Relationship::RELATIONSHIP_FRIENDS.into();
        }
        Ok(())
    }

    async fn relationships(&self, viewer: Uuid, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, (Relationship, i32)>> {
        Ok(user_ids
            .iter()
            .filter(|id| self.are_friends(viewer, **id))
            .map(|id| (*id, (Relationship::RELATIONSHIP_FRIENDS, 0)))
            .collect())
    }

    async fn can_view(&self, viewer: Uuid, owner: Uuid) -> AppResult<bool> {
        Ok(!self.blocked_either(viewer, owner))
    }

    async fn audience_tie(
        &self,
        viewer: Option<Uuid>,
        owner: Uuid,
        circles: &[Uuid],
    ) -> AppResult<Option<tokuly_identity::extras::AudienceTie>> {
        use tokuly_identity::extras::{AudienceTie, Tier};
        let owner_locked = self.locked.lock().unwrap().contains(&owner);
        let Some(viewer) = viewer else { return Ok(Some(AudienceTie { owner_locked, ..Default::default() })) };
        let tier = if self.are_friends(viewer, owner) {
            Tier::Friend
        } else if self.follows.lock().unwrap().contains(&(viewer, owner)) {
            Tier::Follower
        } else {
            Tier::Other
        };
        let close_friend = self.close_friends.lock().unwrap().contains(&(owner, viewer));
        let member = self.circles.lock().unwrap();
        let circles = circles.iter().copied().filter(|c| member.contains(&(*c, viewer))).collect();
        Ok(Some(AudienceTie { tier, owner_locked, close_friend, circles }))
    }

    async fn locked_among(&self, owners: &[Uuid]) -> AppResult<HashSet<Uuid>> {
        let locked = self.locked.lock().unwrap();
        Ok(owners.iter().copied().filter(|o| locked.contains(o)).collect())
    }
}

/// An app whose identity domain runs with `cfg` and a fake social domain.
/// `app.identity` is the testkit's own instance; [`Ext::identity`] is the
/// one serving the RPCs.
pub struct Ext {
    pub app: TestApp,
    pub social: Arc<FakeSocial>,
    pub identity: Identity,
}

pub async fn start(cfg: IdentityConfig) -> Ext {
    let social = Arc::new(FakeSocial::default());
    let served: Arc<OnceLock<Identity>> = Arc::new(OnceLock::new());
    let (social_in, served_in) = (social.clone(), served.clone());
    let app = TestApp::start(MIGRATIONS, move |core, _identity, router| {
        let identity = Identity::new(core.clone(), cfg).expect("identity");
        identity.add_profile_extension(social_in);
        let _ = served_in.set(identity.clone());
        identity.register(router)
    })
    .await;
    let identity = served.get().expect("identity was built").clone();
    Ext { app, social, identity }
}

pub fn unique(prefix: &str) -> String {
    format!("{prefix}{}", tokuly_core::ids::random_base62(8).to_ascii_lowercase())
}

/// A raw answer of a call made like a browser (`x-tokuly-client: web`).
pub struct WebAnswer {
    pub status: u16,
    pub reason: String,
    pub cookies: Vec<String>,
    pub body: serde_json::Value,
}

impl WebAnswer {
    /// The value a `Set-Cookie` of this answer gives `name` (`Some("")` when it clears it).
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.cookies.iter().find_map(|c| {
            let first = c.split(';').next()?;
            let (key, value) = first.split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
    }
}

/// A browser: a cookie jar in front of the identity router, which it reaches
/// below the API's path prefix the way the edge forwards it. Like a real
/// browser, it sends a cookie only to the paths it is scoped to
/// (`Path=`) and never sends a `Secure` one over this plain-HTTP origin.
pub struct Browser {
    router: axum::Router,
    /// Cookie values by name.
    pub jar: HashMap<String, String>,
    /// The `Path` of each cookie the server set (a cookie a test put in the
    /// jar by hand goes everywhere), and the `Secure` ones.
    paths: HashMap<String, String>,
    secure: HashSet<String>,
    /// The API's path prefix (`/api`): the paths browsers see.
    prefix: String,
    ip: std::net::IpAddr,
    /// Send each cookie as a header field of its own, the parked accounts'
    /// first — what an HTTP/2 hop (the edge's h2c to the API) may deliver.
    pub split_cookies: bool,
}

impl Browser {
    pub fn new(ext: &Ext) -> Self {
        let router = axum::Router::new()
            .merge(ext.identity.http_routes())
            .fallback_service(ext.identity.register(connectrpc::Router::new()).into_axum_service())
            .layer(axum::middleware::from_fn_with_state(
                ext.app.core.auth_layer.clone(),
                tokuly_core::auth::auth_middleware,
            ));
        use rand::RngExt as _;
        let mut rng = rand::rng();
        let ip = std::net::IpAddr::from([
            10,
            rng.random_range(0..255u8),
            rng.random_range(0..255u8),
            rng.random_range(1..255u8),
        ]);
        let prefix = ext.app.core.cfg.api_path_prefix().to_owned();
        Self {
            router,
            jar: HashMap::new(),
            paths: HashMap::new(),
            secure: HashSet::new(),
            prefix,
            ip,
            split_cookies: false,
        }
    }

    /// `name=value` of each cookie a browser sends to `path` (below the
    /// prefix, as the router sees it).
    fn cookies_for(&self, path: &str) -> Vec<String> {
        let seen = format!("{}{}", self.prefix, path.split('?').next().unwrap_or_default());
        self.jar
            .iter()
            .filter(|(name, _)| !self.secure.contains(*name))
            .filter(|(name, _)| {
                self.paths.get(*name).is_none_or(|scope| {
                    let scope = scope.trim_end_matches('/');
                    seen == scope || seen.starts_with(&format!("{scope}/"))
                })
            })
            .map(|(k, v)| format!("{k}={v}"))
            .collect()
    }

    /// Follows a link: a `GET` of `path` with the jar's cookies. Keeps the
    /// cookies the answer sets; returns its status, `Location` and the
    /// `Set-Cookie` values.
    pub async fn get(&mut self, path: &str) -> (u16, Option<String>, Vec<String>) {
        self.get_with(path, &[]).await
    }

    /// [`Self::get`] with extra request headers (what a web view adds to its first load).
    pub async fn get_with(&mut self, path: &str, headers: &[(&str, &str)]) -> (u16, Option<String>, Vec<String>) {
        let cookie = self.cookies_for(path);
        let mut request = http::Request::builder()
            .method("GET")
            .uri(path)
            .header("user-agent", "Mozilla/5.0 (Linux; Android 16; Pixel 10) Chrome/157.0.0.0 Mobile Safari/537.36")
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::new(self.ip, 0)));
        if !cookie.is_empty() {
            request = request.header("cookie", cookie.join("; "));
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = self.router.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status().as_u16();
        let location = response.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let cookies: Vec<String> = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_owned))
            .collect();
        self.keep(&cookies);
        (status, location, cookies)
    }

    fn keep(&mut self, cookies: &[String]) {
        for c in cookies {
            let mut parts = c.split(';').map(str::trim);
            let first = parts.next().unwrap_or_default();
            let Some((key, value)) = first.split_once('=') else { continue };
            if value.is_empty() || c.contains("Max-Age=0") {
                self.jar.remove(key);
                self.paths.remove(key);
                self.secure.remove(key);
                continue;
            }
            self.jar.insert(key.to_owned(), value.to_owned());
            let mut path = "/".to_owned();
            self.secure.remove(key);
            for attribute in parts {
                if let Some(value) = attribute.strip_prefix("Path=") {
                    path = value.to_owned();
                } else if attribute.eq_ignore_ascii_case("Secure") {
                    self.secure.insert(key.to_owned());
                }
            }
            self.paths.insert(key.to_owned(), path);
        }
    }

    pub async fn call(&mut self, procedure: &str, access_token: Option<&str>, body: serde_json::Value) -> WebAnswer {
        let cookie = self.cookies_for(&format!("/{procedure}"));
        let mut request = http::Request::builder()
            .method("POST")
            .uri(format!("/{procedure}"))
            .header("content-type", "application/json")
            .header("x-tokuly-client", "web")
            .header("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/157.0.0.0 Safari/537.36")
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::new(self.ip, 0)));
        if self.split_cookies {
            let mut cookie = cookie.clone();
            // `tokuly_ra` before `tokuly_rt`: the order that used to hide the active account.
            cookie.sort();
            for one in cookie {
                request = request.header("cookie", one);
            }
        } else if !cookie.is_empty() {
            request = request.header("cookie", cookie.join("; "));
        }
        if let Some(token) = access_token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let request = request.body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let reason =
            response.headers().get("x-tokuly-error").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
        let cookies: Vec<String> = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_owned))
            .collect();
        self.keep(&cookies);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        WebAnswer { status, reason, cookies, body }
    }
}
