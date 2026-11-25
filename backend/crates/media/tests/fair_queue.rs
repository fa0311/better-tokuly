//! The transcoding queue is fair across accounts: a `media.transcode` job
//! takes the waiting video whose owner has had the smallest share, and an
//! account never runs more than `TOKULY_MEDIA_TRANSCODES_PER_ACCOUNT`
//! transcodes at once. (On its own so that no other test's videos are in
//! the queue: the queue is global.)
mod support;

use support::*;
use tokuly_media::testing::{claim_next_transcode, release_claim};
use tokuly_proto::media::*;
use uuid::Uuid;

async fn upload(h: &Harness, user: &tokuly_testkit::TestUser, clip: &[u8]) -> Uuid {
    asset_id(&h.upload(user, AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO, "clip.mp4", "video/mp4", clip.to_vec()).await)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_accounts_backlog_does_not_hold_up_anyone_elses_video() {
    let h = start().await;
    let core = &h.app.core;
    let (alice, bob) = (h.app.user("alice").await, h.app.user("bob").await);
    let dir = fixture_dir();
    let clip = read(&video(dir.path(), "clip.mp4", VideoSpec { size: "160x90", seconds: 1, rate: 5, audio: false }).await).await;
    // Alice queues three videos; Bob's one arrives after all of them.
    let mut backlog = Vec::new();
    for _ in 0..3 {
        backlog.push(upload(&h, &alice, &clip).await);
    }
    let bobs = upload(&h, &bob, &clip).await;

    // With one of Alice's running, Bob's first video goes before her second.
    assert_eq!(claim_next_transcode(core, 2).await.unwrap(), Some(backlog[0]));
    assert_eq!(claim_next_transcode(core, 2).await.unwrap(), Some(bobs), "Bob does not wait behind Alice's backlog");
    assert_eq!(claim_next_transcode(core, 2).await.unwrap(), Some(backlog[1]));
    // Alice runs two now, her limit: her third waits although a worker is free.
    assert_eq!(claim_next_transcode(core, 2).await.unwrap(), None);
    for id in [backlog[0], bobs, backlog[1]] {
        release_claim(core, id).await.unwrap();
    }

    // The real turns, one after the other: round-robin, every video done.
    // Alice had the last turn above, so Bob, served longest ago, goes first.
    let mut order = Vec::new();
    while let Some(id) = h.media.transcode_next().await.unwrap() {
        order.push(id);
    }
    assert_eq!(order, vec![bobs, backlog[0], backlog[1], backlog[2]]);
    for (user, id) in [(&alice, backlog[0]), (&bob, bobs), (&alice, backlog[2])] {
        let asset = h.get_asset(user, &id.to_string()).await.unwrap();
        assert_eq!(asset.status, AssetStatus::ASSET_STATUS_READY, "{}", asset.error);
    }

    // Workers claiming at the same moment cannot start a third video of one
    // account: the claims are serialized.
    let carol = h.app.user("carol").await;
    let mut carols = Vec::new();
    for _ in 0..5 {
        carols.push(upload(&h, &carol, &clip).await);
    }
    let claims = futures::future::join_all((0..8).map(|_| claim_next_transcode(core, 2))).await;
    let claimed: Vec<Uuid> = claims.into_iter().map(Result::unwrap).flatten().collect();
    assert_eq!(claimed.len(), 2, "two at most per account: {claimed:?}");
    assert!(claimed.iter().all(|id| carols.contains(id)));
}
