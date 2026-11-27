//! The whole path with the real media server: MediaMTX started with
//! `infra/mediamtx/mediamtx.yml`, an encoder (ffmpeg) publishing over RTMP,
//! the auth hook, the reconciler, playback, the viewer count, a viewer
//! being cut off when the stream is restricted, an encoder being
//! disconnected when its stream is deleted, the end and the archive.
//!
//! Skipped (with a message) when `mediamtx` or `ffmpeg` is not installed.
//! The binary is looked for in `TOKULY_MEDIAMTX_BIN`, in
//! `$TOKULY_DEV_HOME/bin` (default `~/tokuly-dev/bin`) and on the PATH. The
//! server listens on free ports picked at start, so the test runs next to the
//! dev infra's MediaMTX and next to itself.
mod common;

use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

use common::*;
use tokuly_live::{LiveConfig, events::ArchiveJob};
use tokuly_proto::live::*;

const SECRET: &str = "e2e-hook-secret-0123456789abcdef";
const API_SECRET: &str = "e2e-api-secret-fedcba9876543210";

/// Kills the process when the test ends, however it ends.
struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn dev_home() -> PathBuf {
    std::env::var_os("TOKULY_DEV_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("tokuly-dev"))
}

fn runs(program: &std::path::Path, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn mediamtx_binary() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("TOKULY_MEDIAMTX_BIN").map(PathBuf::from),
        Some(dev_home().join("bin/mediamtx")),
        Some(PathBuf::from("mediamtx")),
    ];
    candidates.into_iter().flatten().find(|candidate| runs(candidate, "--version"))
}

/// `N` different TCP ports nobody listens on right now, for the media server to take.
fn free_tcp_ports<const N: usize>() -> [u16; N] {
    let held: [_; N] = std::array::from_fn(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap());
    held.map(|socket| socket.local_addr().unwrap().port())
}

/// The same for UDP (SRT and WebRTC's ICE).
fn free_udp_ports<const N: usize>() -> [u16; N] {
    let held: [_; N] = std::array::from_fn(|_| std::net::UdpSocket::bind("127.0.0.1:0").unwrap());
    held.map(|socket| socket.local_addr().unwrap().port())
}

fn encoder(url: &str, seconds: u32) -> Command {
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine",
        ])
        .args(["-c:v", "libx264", "-preset", "veryfast", "-tune", "zerolatency", "-g", "30", "-c:a", "aac"])
        .args(["-t", &seconds.to_string(), "-f", "flv", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Tries `check` until it says yes or `seconds` have passed.
async fn eventually<F, Fut>(seconds: u64, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    loop {
        if check().await {
            return true;
        }
        if tokio::time::Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test]
async fn a_stream_goes_from_encoder_to_archive_through_the_real_media_server() {
    let Some(mediamtx) = mediamtx_binary() else {
        eprintln!(
            "skipped: no mediamtx binary (set TOKULY_MEDIAMTX_BIN or install it into the dev home's bin directory)"
        );
        return;
    };
    if !runs(std::path::Path::new("ffmpeg"), "-version") {
        eprintln!("skipped: ffmpeg is not installed, so nothing can publish a test stream");
        return;
    }
    let [rtmp, hls, webrtc, api, playback] = free_tcp_ports();
    let [srt, ice] = free_udp_ports();
    let recordings = tempfile::tempdir().unwrap();
    let ctx = start_with(LiveConfig {
        mediamtx_api_url: format!("http://127.0.0.1:{api}"),
        hook_secret: SECRET.into(),
        mediamtx_api_secret: API_SECRET.into(),
        rtmp_url: format!("rtmp://127.0.0.1:{rtmp}"),
        srt_url: format!("srt://127.0.0.1:{srt}"),
        whip_url: format!("http://127.0.0.1:{webrtc}"),
        whep_url: format!("http://127.0.0.1:{webrtc}"),
        hls_url: format!("http://127.0.0.1:{hls}"),
        playback_url: format!("http://127.0.0.1:{playback}"),
        recordings_dir: recordings.path().to_string_lossy().into_owned(),
        ..test_config()
    })
    .await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();

    // The hook, served the way the server mounts it.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_port = listener.local_addr().unwrap().port();
    let app = axum::Router::new().nest("/hooks/live", ctx.live.http_routes());
    tokio::spawn(async move { axum::serve(listener, app).await });

    // MediaMTX with the configuration the stack ships, moved onto the ports picked above.
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../infra/mediamtx/mediamtx.yml");
    let log = std::fs::File::create(recordings.path().join("mediamtx.log")).unwrap();
    let _server = Process(
        Command::new(&mediamtx)
            .arg(&config)
            .env("MTX_AUTHHTTPADDRESS", format!("http://mediamtx:{SECRET}@127.0.0.1:{hook_port}/hooks/live/auth"))
            .env("MTX_APIADDRESS", format!("127.0.0.1:{api}"))
            .env("MTX_PLAYBACKADDRESS", format!("127.0.0.1:{playback}"))
            .env("MTX_RTMPADDRESS", format!("127.0.0.1:{rtmp}"))
            .env("MTX_SRTADDRESS", format!("127.0.0.1:{srt}"))
            .env("MTX_HLSADDRESS", format!("127.0.0.1:{hls}"))
            .env("MTX_WEBRTCADDRESS", format!("127.0.0.1:{webrtc}"))
            .env("MTX_WEBRTCLOCALUDPADDRESS", format!("127.0.0.1:{ice}"))
            .env("MTX_PATHDEFAULTS_RECORDPATH", format!("{}/%path/%s-%f", recordings.path().display()))
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .expect("mediamtx starts"),
    );
    // Its control API answers the worker (and, through the hook, nobody else).
    assert!(
        eventually(20, || async { ctx.live.media_paths().await.is_ok() }).await,
        "MediaMTX did not come up on port {api}; its log:\n{}",
        std::fs::read_to_string(recordings.path().join("mediamtx.log")).unwrap_or_default()
    );
    let unauthenticated = reqwest::get(format!("http://127.0.0.1:{api}/v3/paths/list")).await.unwrap();
    assert_eq!(unauthenticated.status().as_u16(), 401);
    // The secret the media server calls the hook with does not open its control API.
    let with_hook_secret = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{api}/v3/paths/list"))
        .basic_auth("tokuly", Some(SECRET))
        .send()
        .await
        .unwrap();
    assert_eq!(with_hook_secret.status().as_u16(), 401);

    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "本番ch").await;
    let stream = ctx.stream(&alice, &channel, "本物の配信").await;
    let info: GetIngestInfoResponse = ctx
        .call(
            STREAMS,
            "GetIngestInfo",
            Some(&alice),
            &GetIngestInfoRequest { stream_id: card(&stream).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let ingest = info.ingest.as_option().unwrap();

    // A wrong key is refused by the media server, and the stream stays off air.
    let wrong = format!("rtmp://127.0.0.1:{rtmp}/live/{}?token=not-the-key", ingest.stream_name);
    let refused = tokio::task::spawn_blocking(move || encoder(&wrong, 3).status().unwrap()).await.unwrap();
    assert!(!refused.success(), "publishing with a wrong key must fail");
    ctx.live.reconcile().await.unwrap();
    assert!(ctx.row(&stream).await.is_upcoming());

    // An encoder that is connected to a stream which is then deleted (before it ever went on
    // air) is disconnected: nothing keeps sending into, and being recorded for, a stream that is gone.
    let preview = ctx.stream_with(&alice, &channel, "消すプレビュー", |r| r.auto_start = Some(false)).await;
    let preview_info: GetIngestInfoResponse = ctx
        .call(
            STREAMS,
            "GetIngestInfo",
            Some(&alice),
            &GetIngestInfoRequest { stream_id: card(&preview).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let _preview_encoder =
        Process(encoder(&preview_info.ingest.as_option().unwrap().rtmp_url, 60).spawn().expect("ffmpeg starts"));
    let connected = eventually(30, || async {
        ctx.live.reconcile().await.unwrap();
        ctx.row(&preview).await.encoder_connected
    })
    .await;
    assert!(connected, "the preview encoder did not connect");
    let preview_path = ctx.row(&preview).await.media_path();
    let _: DeleteStreamResponse = ctx
        .call(
            STREAMS,
            "DeleteStream",
            Some(&alice),
            &DeleteStreamRequest { stream_id: card(&preview).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let kicked =
        eventually(15, || async { !ctx.live.media_path(&preview_path).await.unwrap().is_some_and(|path| path.online) })
            .await;
    assert!(kicked, "the encoder of a deleted stream was left connected");

    // The right key: the encoder connects and the next looks at the media server put the stream on air.
    let _publisher = Process(encoder(&ingest.rtmp_url, 60).spawn().expect("ffmpeg starts"));
    let on_air = eventually(30, || async {
        ctx.live.reconcile().await.unwrap();
        ctx.row(&stream).await.is_live()
    })
    .await;
    assert!(on_air, "the stream did not go on air");
    let row = ctx.row(&stream).await;
    assert!(row.encoder_connected && row.encoder_protocol == "rtmp");

    // Playback: the HLS URL of the page answers with a playlist.
    let page = ctx.get_stream(None, &card(&stream).id).await.unwrap();
    let playback = page.playback.as_option().expect("a stream on air plays");
    let client = reqwest::Client::new();
    let hls_url = playback.hls_url.clone();
    let playlist = eventually(20, || {
        let (client, hls_url) = (client.clone(), hls_url.clone());
        async move {
            match client.get(&hls_url).send().await {
                Ok(response) if response.status().is_success() => {
                    response.text().await.is_ok_and(|body| body.contains("#EXTM3U"))
                }
                _ => false,
            }
        }
    })
    .await;
    assert!(playlist, "the HLS playlist did not answer at {}", playback.hls_url);
    // That reader is a viewer: the media server reports it and the count follows.
    let counted = eventually(15, || async {
        ctx.live.reconcile().await.unwrap();
        ctx.row(&stream).await.viewer_count >= 1
    })
    .await;
    assert!(counted, "the HLS reader was not counted as a viewer");
    assert!(ctx.row(&stream).await.peak_viewer_count >= 1);
    // The recording is being written, and the DVR lists it.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!tokuly_live::recordings::segments(&ctx.live.cfg, &row.short_id).await.is_empty(), "nothing was recorded");
    let listed = client.get(&playback.dvr_list_url).send().await.unwrap();
    assert!(listed.status().is_success(), "the DVR list answered {}", listed.status());

    // Friends only, with two friends watching over HLS. What MediaMTX does (observed with 1.21):
    // it asks the hook once per HLS session, when the multivariant playlist is requested, and
    // serves the playlists and segments of that session on the session's own ID without asking
    // again. So a viewer who loses access is cut on the media server, and the hook refuses
    // their token when they come back.
    let friend = ctx.app.user("friend").await;
    let other_friend = ctx.app.user("friend2").await;
    ctx.befriend(&alice, &friend).await;
    ctx.befriend(&alice, &other_friend).await;
    let friends_only = UpdateStreamRequest {
        stream_id: card(&stream).id.clone(),
        visibility: Some(tokuly_proto::common::Visibility::VISIBILITY_FRIENDS.into()),
        ..Default::default()
    };
    let _: UpdateStreamResponse = ctx.call(STREAMS, "UpdateStream", Some(&alice), &friends_only).await.unwrap();
    // Opens an HLS session with the viewer's own playback URL; returns that URL and the URL
    // of a media playlist inside the session.
    let open_session = |viewer: &tokuly_testkit::TestUser| {
        let (viewer, client, ctx, stream_id) = (viewer.clone(), client.clone(), &ctx, card(&stream).id.clone());
        async move {
            let page = ctx.get_stream(Some(&viewer), &stream_id).await.unwrap();
            let url = page.playback.as_option().unwrap().hls_url.clone();
            assert!(url.contains("?t="), "a friends-only stream plays with a token");
            let index = client.get(&url).send().await.unwrap();
            assert!(index.status().is_success(), "the friend's playlist answered {}", index.status());
            let index = index.text().await.unwrap();
            let media = index.lines().find(|line| !line.starts_with('#') && line.contains(".m3u8")).unwrap().to_owned();
            assert!(media.contains("session="), "MediaMTX serves an HLS session on its own ID: {media}");
            (url.clone(), format!("{}/{media}", url.rsplit_once('/').unwrap().0))
        }
    };
    let plays = |url: &str| {
        let (client, url) = (client.clone(), url.to_owned());
        async move { client.get(&url).send().await.is_ok_and(|response| response.status().is_success()) }
    };
    let (friend_url, friend_session) = open_session(&friend).await;
    let (_, other_session) = open_session(&other_friend).await;
    assert!(plays(&friend_session).await && plays(&other_session).await);
    // The reader IDs MediaMTX lists are the IDs it named when it asked the hook: both
    // sessions are known, with their viewers, and the regular pass leaves them alone.
    let readers = ctx.live.media_path(&row.media_path()).await.unwrap().unwrap().reader_ids;
    assert_eq!(readers.len(), 2, "the plain reader went when the stream was restricted: {readers:?}");
    assert!(readers.iter().all(|(kind, _)| kind == "hlsSession"));
    let known: std::collections::HashMap<String, String> = redis::cmd("HGETALL")
        .arg(format!("live:sessions:{}", row.id))
        .query_async(&mut ctx.app.core.cache.conn())
        .await
        .unwrap();
    for (_, id) in &readers {
        assert!(known.contains_key(id), "MediaMTX lists a reader under another ID than it asked the hook with");
    }
    assert_eq!(ctx.live.enforce_access(&ctx.row(&stream).await, false).await.unwrap(), 0);
    assert!(plays(&friend_session).await && plays(&other_session).await);

    // The friendship ends: that friend's open session stops within the handling of the
    // event, their token (valid for minutes yet) opens no new one, the other friend plays on.
    ctx.unfriend(&alice, &friend).await;
    assert!(!plays(&friend_session).await, "an unfriended viewer's open HLS session still plays");
    assert!(!plays(&friend_url).await, "an unfriended viewer's token still opens a session");
    assert!(plays(&other_session).await, "a friend was cut along with the unfriended viewer");

    // Lost bookkeeping throws nobody out on the regular pass; a pass after a change cuts what
    // it cannot vouch for, and that viewer simply comes back through the hook.
    ctx.app.core.cache.del(&format!("live:sessions:{}", row.id)).await.unwrap();
    assert_eq!(ctx.live.enforce_access(&ctx.row(&stream).await, false).await.unwrap(), 0);
    assert!(plays(&other_session).await);
    assert_eq!(ctx.live.enforce_access(&ctx.row(&stream).await, true).await.unwrap(), 1);
    assert!(!plays(&other_session).await);
    let (_, other_session) = open_session(&other_friend).await;
    assert!(plays(&other_session).await);

    // The stream becomes private while that viewer is watching: their session is cut, the plain
    // URL opens nothing any more, and the owner's own (token-carrying) URL still plays.
    let restrict = UpdateStreamRequest {
        stream_id: card(&stream).id.clone(),
        visibility: Some(tokuly_proto::common::Visibility::VISIBILITY_PRIVATE.into()),
        ..Default::default()
    };
    let _: UpdateStreamResponse = ctx.call(STREAMS, "UpdateStream", Some(&alice), &restrict).await.unwrap();
    let cut = eventually(15, || async {
        ctx.live.media_path(&row.media_path()).await.unwrap().is_some_and(|path| path.reader_ids.is_empty())
    })
    .await;
    assert!(cut, "the viewer of a stream that became private was not cut off");
    let shut_out = client.get(&hls_url).send().await.unwrap();
    assert!(!shut_out.status().is_success(), "the plain URL still plays a private stream ({})", shut_out.status());
    let own = ctx.get_stream(Some(&alice), &card(&stream).id).await.unwrap();
    let own_url = own.playback.as_option().unwrap().hls_url.clone();
    assert!(own_url.contains("?t="));
    let own_plays = eventually(20, || {
        let (client, own_url) = (client.clone(), own_url.clone());
        async move { client.get(&own_url).send().await.is_ok_and(|response| response.status().is_success()) }
    })
    .await;
    assert!(own_plays, "the owner can no longer watch their own private stream");
    // The owner's session came in with their token, so it is theirs to keep.
    assert_eq!(ctx.live.enforce_access(&ctx.row(&stream).await, true).await.unwrap(), 0);
    assert!(!plays(&other_session).await, "a friend still watches a stream that became private");

    // The end: the encoder is disconnected and the path goes away.
    let ended = ctx.end(&alice, &stream).await;
    assert_eq!(card(&ended).status, StreamStatus::STREAM_STATUS_ENDED);
    let name = row.media_path();
    let gone =
        eventually(15, || async { !ctx.live.media_path(&name).await.unwrap().is_some_and(|path| path.online) }).await;
    assert!(gone, "the encoder was not disconnected");
    // Publishing into a stream that is over is refused.
    let late_url = ingest.rtmp_url.clone();
    let late = tokio::task::spawn_blocking(move || encoder(&late_url, 3).status().unwrap()).await.unwrap();
    assert!(!late.success());

    // The archive job turns the recording into a video on the channel and removes the segments.
    ctx.live.archive(ArchiveJob { stream_id: row.id }).await.unwrap();
    let archived = ctx.get_stream(Some(&alice), &card(&stream).id).await.unwrap();
    assert_eq!(
        archived.archive_status,
        ArchiveStatus::ARCHIVE_STATUS_READY,
        "{}",
        archived.studio.as_option().unwrap().archive_error
    );
    let video = tokuly_video::videos::find_ref(&ctx.app.core, &archived.archive_video_id)
        .await
        .unwrap()
        .expect("the archive video");
    ctx.media.process_now(video.asset_id).await.unwrap();
    let asset = tokuly_media::assets::find(&ctx.app.core, video.asset_id).await.unwrap().unwrap();
    assert!(asset.is_ready(), "the archive could not be processed: {}", asset.error);
    assert!(asset.video().unwrap().duration_seconds > 3.0);
    assert!(tokuly_live::recordings::segments(&ctx.live.cfg, &row.short_id).await.is_empty());
}
