//! LiveKit's webhook, end to end: a LiveKit server of the test's own is
//! told to post to this domain's route, and what arrives carries LiveKit's
//! real signature. (The shared development server cannot be pointed at a
//! test; `tools/dev/infra.sh` and `infra/livekit/livekit.yaml` declare the
//! same `webhook:` block for the real API.)
mod common;

use std::{
    net::{TcpListener, UdpSocket},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use common::*;
use tokuly_core::livekit::LiveKit;
use tokuly_proto::spaces::*;
use tokuly_spaces::SpacesConfig;

const KEY: &str = "hooktest";
const SECRET: &str = "a-webhook-test-secret-of-thirty-two-chars";

/// The LiveKit server binary of the development setup, if there is one.
fn livekit_binary() -> Option<PathBuf> {
    let home = std::env::var("TOKULY_DEV_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|home| PathBuf::from(home).join("tokuly-dev")))
        .ok()?;
    let binary = home.join("bin/livekit-server");
    binary.is_file().then_some(binary)
}

fn free_tcp_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Stops the server when the test is over, however it ends.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn livekit_itself_calls_the_webhook_and_its_signature_is_accepted() {
    let Some(binary) = livekit_binary() else {
        eprintln!("skipped: no livekit-server binary (tools/dev/infra.sh install)");
        return;
    };
    // Where LiveKit will post: this test's own listener, serving the domain's route.
    let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sink_port = sink.local_addr().unwrap().port();
    let (port, rtc_tcp, rtc_udp) = (free_tcp_port(), free_tcp_port(), free_udp_port());
    let config = format!(
        "port: {port}\nbind_addresses: [\"127.0.0.1\"]\nrtc:\n  tcp_port: {rtc_tcp}\n  udp_port: {rtc_udp}\n  use_external_ip: false\n\
         keys:\n  {KEY}: {SECRET}\nwebhook:\n  api_key: {KEY}\n  urls:\n    - http://127.0.0.1:{sink_port}/spaces/livekit/webhook\n\
         logging:\n  level: warn\n"
    );
    let _server = Server(
        Command::new(binary)
            .args(["--config-body", &config])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("livekit-server starts"),
    );
    let livekit =
        LiveKit::new(&format!("http://127.0.0.1:{port}"), &format!("ws://127.0.0.1:{port}"), KEY, SECRET).unwrap();
    for _ in 0..100 {
        if livekit.list_rooms(&[]).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    livekit.list_rooms(&[]).await.expect("the test's LiveKit answers");

    // No patience and no timer: only the webhook can make anything happen here.
    let cfg = SpacesConfig { connect_grace_seconds: 0, reconnect_grace_seconds: 0, ..Default::default() };
    let ctx = start_on(cfg, livekit).await;
    let answers: Arc<Mutex<Vec<u16>>> = Arc::default();
    let seen = answers.clone();
    let routes =
        ctx.spaces.http_routes().layer(axum::middleware::map_response(move |response: axum::response::Response| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(response.status().as_u16());
                response
            }
        }));
    tokio::spawn(async move { axum::serve(sink, routes).await });

    let host = ctx.app.user("taro").await;
    let banned = ctx.app.user("jiro").await;
    let (room, host_credentials) = ctx.open_room(&host, "出禁", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let _host_connection = ctx.connect(&host_credentials).await;
    let credentials = ctx.join(&banned, &room.id).await.credentials.as_option().unwrap().clone();
    let remove = RemoveParticipantRequest {
        space_id: room.id.clone(),
        user_id: banned.id.to_string(),
        ban: true,
        ..Default::default()
    };
    let _: RemoveParticipantResponse = ctx.call("Space", "RemoveParticipant", &host, &remove).await.unwrap();

    // The room ends; the banned person still holds a token, and LiveKit
    // would open the room again for them. LiveKit says so (`room_started`)
    // and the connection is cut without anyone here lifting a finger.
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &host, &EndSpaceRequest { space_id: room.id.clone(), ..Default::default() })
        .await
        .unwrap();
    answers.lock().unwrap().clear();
    let (socket, _) = tokio_tungstenite::connect_async(format!(
        "{}/rtc?access_token={}&auto_subscribe=1&protocol=16&sdk=rust&version=0.1",
        credentials.url, credentials.token
    ))
    .await
    .expect("LiveKit still accepts the token");
    let mut gone = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let delivered = !answers.lock().unwrap().is_empty();
        if delivered && ctx.connected(&credentials.room, &banned.id.to_string()).await.is_none() {
            gone = true;
            break;
        }
    }
    drop(socket);
    let answers = answers.lock().unwrap().clone();
    assert!(!answers.is_empty(), "LiveKit never called the webhook");
    assert!(answers.iter().all(|status| *status == 200), "LiveKit's own signature was refused: {answers:?}");
    assert!(gone, "the connection without a seat was not cut");
}
