//! An account that had a turn before is not starved by a stream of new
//! accounts: among equal shares, whoever has waited longest goes first.
//! (Its own binary: the transcoding queue is global.)
mod support;

use support::*;
use tokuly_proto::media::*;
use uuid::Uuid;

async fn upload(h: &Harness, user: &tokuly_testkit::TestUser, clip: &[u8]) -> Uuid {
    asset_id(&h.upload(user, AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO, "clip.mp4", "video/mp4", clip.to_vec()).await)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn new_accounts_arriving_later_do_not_jump_a_returning_account() {
    let h = start().await;
    let dir = fixture_dir();
    let clip = read(&video(dir.path(), "clip.mp4", VideoSpec { size: "160x90", seconds: 1, rate: 5, audio: false }).await).await;

    // Ren had a turn a moment ago.
    let ren = h.app.user("ren").await;
    let first = upload(&h, &ren, &clip).await;
    assert_eq!(h.media.transcode_next().await.unwrap(), Some(first));

    // Ren's next video waits; then new accounts keep arriving.
    let waiting = upload(&h, &ren, &clip).await;
    let mut newcomers = Vec::new();
    for name in ["a", "b", "c"] {
        let user = h.app.user(name).await;
        newcomers.push(upload(&h, &user, &clip).await);
    }
    // Ren waited longest, so Ren goes first; the newcomers then follow in
    // the order they arrived, each having waited since their upload.
    let mut order = Vec::new();
    while let Some(id) = h.media.transcode_next().await.unwrap() {
        order.push(id);
    }
    assert_eq!(order, [vec![waiting], newcomers].concat());
}
