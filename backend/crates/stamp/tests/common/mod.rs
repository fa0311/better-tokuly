//! Shared fixtures for the stamp integration tests.
//!
//! Every test starts its own app ([`start`]) against the test binary's
//! database, creates its own users, and — because store shelves are global —
//! tags its packs with a tag nobody else uses ([`unique`]).
#![allow(dead_code)]

use serde::{Serialize, de::DeserializeOwned};
use tokuly_proto::stamp::*;
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_testkit::{RpcError, TestApp, TestUser};
use uuid::Uuid;

pub const MIGRATIONS: &[&str] = &["0001", "0100", "0400", "0500", "1400"];

/// A running app plus the stamp domain behind it, for calling the
/// cross-domain API and delivering events by hand.
pub struct Ctx {
    pub app: TestApp,
    pub stamp: Stamp,
}

pub fn test_config() -> StampConfig {
    StampConfig { seed_official: false, ..Default::default() }
}

pub async fn start() -> Ctx {
    start_with(test_config()).await
}

pub async fn start_with(cfg: StampConfig) -> Ctx {
    let mut domain = None;
    let app = TestApp::start(MIGRATIONS, |core, identity, router| {
        let stamp = Stamp::new(core.clone(), identity.clone(), cfg).expect("stamp domain");
        let router = stamp.register(router);
        domain = Some(stamp);
        router
    })
    .await;
    Ctx { app, stamp: domain.expect("register ran") }
}

/// A lowercase string no other test uses, e.g. for tags and searchable names.
pub fn unique(prefix: &str) -> String {
    format!("{prefix}{}", tokuly_core::ids::random_base62(10).to_ascii_lowercase())
}

/// The kinds of processed upload a stamp can be made from, plus ones it cannot.
#[derive(Clone, Copy)]
pub enum Upload {
    Still,
    Animated,
    Lottie,
    /// A video uploaded for stamps (never a valid stamp).
    Video,
    /// A still image uploaded for chat, not for stamps.
    ChatImage,
    /// A still image whose processing has not finished.
    Processing,
}

impl Ctx {
    /// Calls `tokuly.stamp.v1.Stamp<service>Service/<method>`.
    pub async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        service: &str,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.stamp.v1.Stamp{service}Service/{method}"), user, request).await
    }

    /// The reason an RPC is rejected with.
    pub async fn reason<Req: Serialize>(
        &self,
        service: &str,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> String {
        match self.call::<_, serde_json::Value>(service, method, user, request).await {
            Ok(response) => panic!("{service}/{method} should fail, got {response}"),
            Err(error) => error.reason,
        }
    }

    /// Inserts a media asset as the media domain would leave it after
    /// processing, and returns its ID.
    pub async fn upload(&self, owner: &TestUser, upload: Upload) -> String {
        // (purpose, kind, status, content type, file name, has image)
        let (purpose, kind, status, content_type, file_name, has_image) = match upload {
            Upload::Still => (11, 1, 3, "image/webp", "stamp.png", true),
            Upload::Animated => (11, 5, 3, "image/webp", "stamp.gif", true),
            Upload::Lottie => (11, 4, 3, "application/zip+dotlottie", "stamp.lottie", false),
            Upload::Video => (11, 2, 3, "video/mp4", "stamp.mp4", false),
            Upload::ChatImage => (4, 1, 3, "image/webp", "photo.jpg", true),
            Upload::Processing => (11, 1, 2, "image/png", "stamp.png", false),
        };
        let id = Uuid::now_v7();
        let url = format!("https://media.test/{id}/main");
        let image = has_image.then(|| {
            serde_json::json!({
                "assetId": id.to_string(),
                "url": format!("https://media.test/{id}/512.webp"),
                "width": 512,
                "height": 448,
                "thumbhash": "1QcSHQRnh493V4dIh4eXh1h4kJUI",
                "variants": [{ "size": 160, "url": format!("https://media.test/{id}/160.webp") }],
            })
        });
        sqlx::query(
            "INSERT INTO media_assets (id, owner_id, purpose, kind, status, content_type, file_name, original_key, url, image, ready_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, CASE WHEN $5 = 3 THEN now() END)",
        )
        .bind(id)
        .bind(owner.id)
        .bind(purpose)
        .bind(kind)
        .bind(status)
        .bind(content_type)
        .bind(file_name)
        .bind(format!("originals/{id}"))
        .bind(if status == 3 { url } else { String::new() })
        .bind(image)
        .execute(&self.app.core.db)
        .await
        .expect("insert media asset");
        id.to_string()
    }

    pub async fn new_stamp(&self, owner: &TestUser, name: &str) -> NewStamp {
        NewStamp { asset_id: self.upload(owner, Upload::Still).await, name: name.to_owned(), ..Default::default() }
    }

    /// Creates a draft pack.
    pub async fn draft(&self, creator: &TestUser, name: &str, price: i32, tags: &[&str]) -> Pack {
        let request = CreatePackRequest {
            name: name.to_owned(),
            description: format!("{name} の説明"),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            price,
            ..Default::default()
        };
        let response: CreatePackResponse = self.call("Studio", "CreatePack", Some(creator), &request).await.unwrap();
        response.pack.into_option().unwrap()
    }

    /// Adds `count` still stamps named `<prefix>1`, `<prefix>2`…
    pub async fn add_stamps(&self, creator: &TestUser, pack_id: &str, prefix: &str, count: usize) -> Pack {
        let mut stamps = Vec::new();
        for n in 1..=count {
            stamps.push(self.new_stamp(creator, &format!("{prefix}{n}")).await);
        }
        let request = AddStampsRequest { pack_id: pack_id.to_owned(), stamps, ..Default::default() };
        let response: AddStampsResponse = self.call("Studio", "AddStamps", Some(creator), &request).await.unwrap();
        response.pack.into_option().unwrap()
    }

    pub async fn submit(&self, creator: &TestUser, pack_id: &str) -> Pack {
        let response: SubmitPackResponse = self
            .call(
                "Studio",
                "SubmitPack",
                Some(creator),
                &SubmitPackRequest { pack_id: pack_id.to_owned(), ..Default::default() },
            )
            .await
            .unwrap();
        response.pack.into_option().unwrap()
    }

    pub async fn moderator(&self) -> TestUser {
        self.app.user_with_roles("moderator", &["moderator"]).await
    }

    pub async fn approve(&self, pack_id: &str) -> Pack {
        let moderator = self.moderator().await;
        let response: ApprovePackResponse = self
            .call(
                "Review",
                "ApprovePack",
                Some(&moderator),
                &ApprovePackRequest { pack_id: pack_id.to_owned(), ..Default::default() },
            )
            .await
            .unwrap();
        response.pack.into_option().unwrap()
    }

    /// A pack with `stamps` stamps that went through review and is in the store.
    pub async fn published(&self, creator: &TestUser, name: &str, price: i32, tags: &[&str], stamps: usize) -> Pack {
        let pack = self.draft(creator, name, price, tags).await;
        let id = pack.summary.id.clone();
        self.add_stamps(creator, &id, "スタンプ", stamps).await;
        self.submit(creator, &id).await;
        self.approve(&id).await
    }

    /// Takes a pack out of the store (or out of review) as its creator.
    pub async fn unpublish(&self, creator: &TestUser, pack_id: &str) -> Pack {
        let request = UnpublishPackRequest { pack_id: pack_id.to_owned(), ..Default::default() };
        let response: UnpublishPackResponse =
            self.call("Studio", "UnpublishPack", Some(creator), &request).await.unwrap();
        response.pack.into_option().unwrap()
    }

    pub async fn remove_stamp(&self, creator: &TestUser, stamp_id: &str) -> Pack {
        let request = RemoveStampRequest { stamp_id: stamp_id.to_owned(), ..Default::default() };
        let response: RemoveStampResponse = self.call("Studio", "RemoveStamp", Some(creator), &request).await.unwrap();
        response.pack.into_option().unwrap()
    }

    pub async fn delete_pack(&self, creator: &TestUser, pack_id: &str) {
        let request = DeletePackRequest { pack_id: pack_id.to_owned(), ..Default::default() };
        let _: DeletePackResponse = self.call("Studio", "DeletePack", Some(creator), &request).await.unwrap();
    }

    /// Flags (or unflags) a creator as trusted, as `moderator`.
    pub async fn trust(&self, moderator: &TestUser, creator: &TestUser, trusted: bool) {
        let request = SetCreatorTrustedRequest { user_id: creator.id.to_string(), trusted, ..Default::default() };
        let _: SetCreatorTrustedResponse =
            self.call("Review", "SetCreatorTrusted", Some(moderator), &request).await.unwrap();
    }

    pub async fn get_pack(&self, viewer: Option<&TestUser>, pack_id: &str) -> Result<Pack, RpcError> {
        let response: GetPackResponse = self
            .call("Store", "GetPack", viewer, &GetPackRequest { pack_id: pack_id.to_owned(), ..Default::default() })
            .await?;
        Ok(response.pack.into_option().unwrap())
    }

    pub async fn install(&self, user: &TestUser, pack_id: &str) -> PackSummary {
        let response: InstallPackResponse = self
            .call(
                "Library",
                "InstallPack",
                Some(user),
                &InstallPackRequest { pack_id: pack_id.to_owned(), ..Default::default() },
            )
            .await
            .unwrap();
        response.pack.into_option().unwrap()
    }

    pub async fn uninstall(&self, user: &TestUser, pack_id: &str) {
        let request = UninstallPackRequest { pack_id: pack_id.to_owned(), ..Default::default() };
        let _: UninstallPackResponse = self.call("Library", "UninstallPack", Some(user), &request).await.unwrap();
    }

    /// Buys a pack at the price the user expects.
    pub async fn buy(&self, user: &TestUser, pack_id: &str, expected_price: i32) -> PurchasePackResponse {
        let request = PurchasePackRequest { pack_id: pack_id.to_owned(), expected_price, ..Default::default() };
        self.call("Library", "PurchasePack", Some(user), &request).await.unwrap()
    }

    pub async fn favorite(&self, user: &TestUser, stamp_id: &str) {
        let request = FavoriteStampRequest { stamp_id: stamp_id.to_owned(), ..Default::default() };
        let _: FavoriteStampResponse = self.call("Library", "FavoriteStamp", Some(user), &request).await.unwrap();
    }

    /// Saves a received stamp.
    pub async fn save(&self, user: &TestUser, stamp_id: &str) -> SaveStampResponse {
        let request = SaveStampRequest { stamp_id: stamp_id.to_owned(), ..Default::default() };
        self.call("Library", "SaveStamp", Some(user), &request).await.unwrap()
    }

    /// Creates a personal stamp from a fresh still upload.
    pub async fn personal(&self, owner: &TestUser, name: &str) -> tokuly_proto::stamp::Stamp {
        let request = CreatePersonalStampRequest {
            asset_id: self.upload(owner, Upload::Still).await,
            name: name.to_owned(),
            ..Default::default()
        };
        let response: CreatePersonalStampResponse =
            self.call("Library", "CreatePersonalStamp", Some(owner), &request).await.unwrap();
        response.stamp.into_option().unwrap()
    }

    /// Gives a user coins to spend.
    pub async fn grant_coins(&self, user: &TestUser, amount: i64) {
        tokuly_wallet::ledger::grant(&self.app.core, user.id, amount, "test", &unique("grant:"), serde_json::json!({}))
            .await
            .unwrap();
    }

    pub async fn balance(&self, user: &TestUser) -> tokuly_wallet::ledger::Balance {
        tokuly_wallet::ledger::balance(&self.app.core, user.id).await.unwrap()
    }

    /// The names under which an upload is held in the media domain, sorted.
    pub async fn holders(&self, asset_id: Uuid) -> Vec<String> {
        sqlx::query_scalar("SELECT holder FROM media_asset_holders WHERE asset_id = $1 ORDER BY holder")
            .bind(asset_id)
            .fetch_all(&self.app.core.db)
            .await
            .expect("read asset holders")
    }

    /// Whether an upload still exists in the media domain.
    pub async fn upload_exists(&self, asset_id: Uuid) -> bool {
        tokuly_media::assets::find(&self.app.core, asset_id).await.expect("read asset").is_some()
    }

    /// Notification requests of `kind` addressed to `user`.
    pub async fn notifications(&self, user: &TestUser, kind: &str) -> Vec<serde_json::Value> {
        self.app
            .events("notify.requested")
            .await
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data["user_id"] == user.id.to_string() && data["kind"] == kind)
            .collect()
    }

    /// Search index changes for a pack, oldest first: `true` = indexed, `false` = removed.
    pub async fn search_changes(&self, pack_id: &str) -> Vec<bool> {
        self.app
            .events("search.changed")
            .await
            .into_iter()
            .filter(|event| event.data["id"] == pack_id)
            .map(|event| !event.data["doc"].is_null())
            .collect()
    }
}

/// An event as a consumer receives it, for delivering by hand (no relay runs in tests).
pub fn event(topic: &str, data: serde_json::Value) -> tokuly_core::events::Event {
    tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: topic.to_owned(),
        actor_id: None,
        occurred_at: chrono::Utc::now(),
        data,
    }
}

/// The upload a stamp was made from.
pub fn asset_of(stamp: &tokuly_proto::stamp::Stamp) -> Uuid {
    stamp.image.asset_id.parse().expect("stamps made from uploads carry the asset ID")
}

/// The name under which a stamp holds its upload.
pub fn hold_of(stamp: &tokuly_proto::stamp::Stamp) -> String {
    format!("stamp:{}", stamp.stamp_ref.id)
}

/// The ID of a pack (as returned by the API).
pub fn id_of(pack: &Pack) -> String {
    pack.summary.id.clone()
}

/// The UUID of the `index`-th stamp of a pack.
pub fn stamp_uuid(pack: &Pack, index: usize) -> Uuid {
    pack.stamps[index].stamp_ref.id.parse().unwrap()
}

pub fn stamp_names(stamps: &[tokuly_proto::stamp::Stamp]) -> Vec<String> {
    stamps.iter().map(|stamp| stamp.stamp_ref.name.clone()).collect()
}

pub fn pack_names(packs: &[PackSummary]) -> Vec<String> {
    packs.iter().map(|pack| pack.name.clone()).collect()
}
