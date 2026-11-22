//! Following the media server: the reconciler brings the streams in line
//! with the paths MediaMTX reports.
//!
//! A look at the media server concerns every stream there is, so these
//! tests have a test binary (and with it a database) of their own and take
//! turns.
mod common;

use std::time::Duration;

use common::*;
use tokuly_live::{LiveConfig, hooks::AuthRequest, lifecycle::Reconciled};
use tokuly_proto::live::*;

/// One test at a time: each one's look at the media server would end the
/// other's streams.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A media server that is configured (but, here, never reachable).
fn with_media_server() -> LiveConfig {
    LiveConfig {
        mediamtx_api_url: "http://127.0.0.1:9".into(),
        hook_secret: "test-hook-secret-0123456789".into(),
        mediamtx_api_secret: "test-api-secret-9876543210".into(),
        ..test_config()
    }
}

#[tokio::test]
async fn the_reconciler_follows_the_encoder() {
    let _turn = TURN.lock().await;
    let ctx = start_with(LiveConfig { reconnect_grace_seconds: 2, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "エンコーダch").await;
    let stream = ctx.stream(&alice, &channel, "自動で始まる").await;
    let manual = ctx.stream_with(&alice, &channel, "手動で始める", |r| r.auto_start = Some(false)).await;
    let short_id = card(&stream).short_id.clone();
    let mut feed = ctx.app.listen_user(alice.id).await;
    let mut topic = ctx.app.listen_topic(&stream.realtime_topic).await;

    // The encoders connect: the first stream goes on air by itself, the second waits for its owner.
    let paths = vec![online_path(&short_id, 0), online_path(&card(&manual).short_id, 0)];
    let report = ctx.live.apply_media_state(Some(&paths)).await.unwrap();
    assert_eq!((report.connected, report.started), (2, 1));
    let state = wait_for(&mut feed, |push| match push {
        Push::EncoderState(state) => Some(state),
        _ => None,
    })
    .await;
    assert!(state.connected && state.protocol == "rtmp");
    let went_live = wait_for(&mut topic, |push| match push {
        Push::StreamUpdated(summary) => Some(summary),
        _ => None,
    })
    .await;
    assert_eq!(went_live.status, StreamStatus::STREAM_STATUS_LIVE);
    let row = ctx.row(&stream).await;
    assert!(row.is_live() && row.encoder_connected && row.started_at.is_some());
    assert_eq!(
        ctx.app
            .events_by("live.stream.started", alice.id)
            .await
            .iter()
            .filter(|e| e.data["short_id"] == short_id.as_str())
            .count(),
        1
    );
    let waiting = ctx.get_stream(Some(&alice), &card(&manual).id).await.unwrap();
    assert_eq!(card(&waiting).status, StreamStatus::STREAM_STATUS_UPCOMING);
    assert!(waiting.studio.as_option().unwrap().encoder_connected);

    // The same look again, twice at once: nothing changes, nothing is announced twice (checklist 6, 10).
    let (a, b) = tokio::join!(ctx.live.apply_media_state(Some(&paths)), ctx.live.apply_media_state(Some(&paths)));
    assert_eq!((a.unwrap().started, b.unwrap().started), (0, 0));
    assert_eq!(
        ctx.app
            .events_by("live.stream.started", alice.id)
            .await
            .iter()
            .filter(|e| e.data["short_id"] == short_id.as_str())
            .count(),
        1
    );
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM live_ingest_sessions WHERE stream_id = $1")
        .bind(sid(&stream))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(sessions, 1);

    // The media server could not be asked: nothing is assumed.
    assert_eq!(ctx.live.apply_media_state(None).await.unwrap(), Reconciled::default());
    assert!(ctx.row(&stream).await.encoder_connected);

    // The encoder goes away and comes back within the grace period: still on air, a second session.
    let report = ctx.live.apply_media_state(Some(&paths[1..])).await.unwrap();
    assert_eq!((report.disconnected, report.ended), (1, 0));
    assert!(ctx.row(&stream).await.encoder_lost_at.is_some());
    let report = ctx.live.apply_media_state(Some(&paths)).await.unwrap();
    assert_eq!(report.connected, 1);
    let row = ctx.row(&stream).await;
    assert!(row.is_live() && row.encoder_lost_at.is_none());
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM live_ingest_sessions WHERE stream_id = $1")
        .bind(sid(&stream))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(sessions, 2);

    // It goes away for good (the media server restarted, say): after the grace period the stream ends.
    ctx.live.apply_media_state(Some(&[])).await.unwrap();
    assert!(ctx.row(&stream).await.is_live(), "the grace period is running");
    tokio::time::sleep(Duration::from_millis(2300)).await;
    let report = ctx.live.apply_media_state(Some(&[])).await.unwrap();
    assert!(report.ended >= 1);
    let ended = ctx.get_stream(Some(&alice), &card(&stream).id).await.unwrap();
    assert_eq!(card(&ended).status, StreamStatus::STREAM_STATUS_ENDED);
    assert_eq!(ended.studio.as_option().unwrap().end_reason, EndReason::END_REASON_ENCODER_GONE);
    assert_eq!(ended.archive_status, ArchiveStatus::ARCHIVE_STATUS_PENDING);
    // An upcoming stream whose encoder left just waits again.
    let waiting = ctx.row(&manual).await;
    assert!(waiting.is_upcoming() && !waiting.encoder_connected);
}

#[tokio::test]
async fn a_stream_without_auto_start_goes_on_air_when_its_owner_says_so() {
    let _turn = TURN.lock().await;
    let ctx = start_with(with_media_server()).await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "手動ch").await;
    let stream = ctx.stream_with(&alice, &channel, "プレビューしてから", |r| r.auto_start = Some(false)).await;
    let start = StartStreamRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(STREAMS, "StartStream", Some(&bob), &start).await, "STREAM_NOT_FOUND");
    ctx.live.apply_media_state(Some(&[online_path(&card(&stream).short_id, 0)])).await.unwrap();
    // The owner previews (with a token); nobody else can play yet.
    let preview = ctx.get_stream(Some(&alice), &card(&stream).id).await.unwrap();
    let playback = preview.playback.as_option().expect("the owner previews");
    assert!(!playback.token.is_empty() && playback.hls_url.contains("?t="));
    assert!(!ctx.get_stream(Some(&bob), &card(&stream).id).await.unwrap().playback.is_set());
    // A playback token is a credential for the media server: a caller with an API token is
    // not handed one, not even the owner's own.
    let row = ctx.row(&stream).await;
    assert!(ctx.live.playback_for(&row, Some(alice.id), true).is_some_and(|playback| !playback.token.is_empty()));
    assert!(ctx.live.playback_for(&row, Some(alice.id), false).is_none());
    let read = AuthRequest {
        action: "read".into(),
        path: format!("live/{}", card(&stream).short_id),
        protocol: "hls".into(),
        ..Default::default()
    };
    assert!(
        ctx.live.authorize_media(&read).await.unwrap().is_err(),
        "not on air: no reading without the owner's token"
    );
    assert_eq!(
        ctx.live
            .authorize_media(&AuthRequest { query: format!("t={}", playback.token), ..read.clone() })
            .await
            .unwrap(),
        Ok(())
    );

    let started: StartStreamResponse = ctx.call(STREAMS, "StartStream", Some(&alice), &start).await.unwrap();
    assert_eq!(card(started.stream.as_option().unwrap()).status, StreamStatus::STREAM_STATUS_LIVE);
    // Starting again is a no-op; now anyone plays, without a token.
    let _: StartStreamResponse = ctx.call(STREAMS, "StartStream", Some(&alice), &start).await.unwrap();
    assert_eq!(ctx.app.events_by("live.stream.started", alice.id).await.len(), 1);
    let seen = ctx.get_stream(None, &card(&stream).id).await.unwrap();
    let playback = seen.playback.as_option().unwrap();
    assert!(
        playback.token.is_empty()
            && playback.hls_url.ends_with(&format!("/live/{}/index.m3u8", card(&stream).short_id))
    );
    // What anyone may watch, an API token may be told the URLs of.
    let row = ctx.row(&stream).await;
    assert!(ctx.live.playback_for(&row, Some(bob.id), false).is_some_and(|playback| playback.token.is_empty()));
    assert!(
        playback.whep_url.ends_with("/whep")
            && playback.dvr_window_seconds == 3600
            && playback.dvr_get_url.contains("/get?path=live/")
    );
    assert_eq!(ctx.live.authorize_media(&read).await.unwrap(), Ok(()));
    ctx.end(&alice, &stream).await;
    assert_eq!(ctx.reason(STREAMS, "StartStream", Some(&alice), &start).await, "STREAM_ENDED");
}
