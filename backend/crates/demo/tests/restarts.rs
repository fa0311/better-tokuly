//! Starting the demo again and again: the bots keep their tokens, and the
//! operators' allowance of new tokens (20 an hour) is never used up.
//!
//! The server is a stand-in that answers what setting the cast up asks,
//! with the developer platform's rules for bot tokens: twenty created per
//! operator an hour, and a revoked token no longer works.
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use serde::{Serialize, de::DeserializeOwned};
use tokuly_demo::{DemoConfig, api::Api, config::Center, script::Script, setup, town::LatLng};
use tokuly_proto::{
    common::UserSummary as User,
    dev::{
        AccessToken, Bot, CreateBotRequest, CreateBotResponse, CreateBotTokenRequest, CreateBotTokenResponse,
        ListBotTokensRequest, ListBotTokensResponse, ListBotsResponse, RevokeBotTokenRequest,
    },
    identity::{AuthResponse, GetMeResponse, Me, SignInRequest},
    notes::GetNoteRulesResponse,
};

const TOKENS_AN_HOUR: usize = 20;
const BOTS_PER_OPERATOR: usize = 5;

#[derive(Default)]
struct Server {
    /// Operator address → its bots' IDs.
    operators: HashMap<String, Vec<String>>,
    /// Session → operator address.
    sessions: HashMap<String, String>,
    /// (token ID, bot ID, secret, live).
    tokens: Vec<(String, String, String, bool)>,
    /// Tokens each operator created (the hour never ends in this test).
    created: HashMap<String, usize>,
}

type Shared = Arc<Mutex<Server>>;

fn refusal(status: StatusCode, reason: &str) -> Response {
    let code = if status == StatusCode::UNAUTHORIZED { "unauthenticated" } else { "resource_exhausted" };
    (status, [("x-tokuly-error", reason)], format!(r#"{{"code":"{code}","message":"{reason}"}}"#)).into_response()
}

fn answer(message: impl Serialize) -> Response {
    ([("content-type", "application/json")], serde_json::to_vec(&message).unwrap()).into_response()
}

fn read<T: DeserializeOwned>(body: &Bytes) -> T {
    serde_json::from_slice(body).unwrap()
}

async fn rpc(State(server): State<Shared>, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned();
    let mut server = server.lock().unwrap();
    let procedure = uri.path().rsplit('/').next().unwrap_or_default();
    // Calls made as a bot: its token must be live.
    if matches!(procedure, "GetMe" | "GetNoteRules" | "SendFriendRequest") {
        let Some((_, bot, _, _)) = server.tokens.iter().find(|(_, _, secret, live)| *live && *secret == bearer) else {
            return refusal(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
        };
        let user =
            User { id: bot.clone(), avatar: tokuly_proto::common::Image::default().into(), ..Default::default() };
        return match procedure {
            "GetMe" => answer(GetMeResponse {
                me: Me { user: user.into(), ..Default::default() }.into(),
                ..Default::default()
            }),
            "GetNoteRules" => answer(GetNoteRulesResponse { agreed: true, ..Default::default() }),
            _ => answer(serde_json::json!({})),
        };
    }
    if procedure == "SignIn" {
        let request: SignInRequest = read(&body);
        let session = format!("session-{}", server.sessions.len());
        server.operators.entry(request.identifier.clone()).or_default();
        server.sessions.insert(session.clone(), request.identifier);
        return answer(AuthResponse { access_token: session, ..Default::default() });
    }
    let Some(operator) = server.sessions.get(&bearer).cloned() else {
        return refusal(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
    };
    match procedure {
        "SignOut" | "UpdateBot" => answer(serde_json::json!({})),
        "ListBots" => {
            let bots = server.operators[&operator]
                .iter()
                .map(|id| Bot { user: User { id: id.clone(), ..Default::default() }.into(), ..Default::default() })
                .collect();
            answer(ListBotsResponse { bots, max_bots: BOTS_PER_OPERATOR as i32, ..Default::default() })
        }
        "CreateBot" => {
            let _: CreateBotRequest = read(&body);
            let id = format!("bot-{operator}-{}", server.operators[&operator].len());
            server.operators.get_mut(&operator).unwrap().push(id.clone());
            let bot = Bot { user: User { id, ..Default::default() }.into(), ..Default::default() };
            answer(CreateBotResponse { bot: bot.into(), ..Default::default() })
        }
        "ListBotTokens" => {
            let request: ListBotTokensRequest = read(&body);
            let tokens = server
                .tokens
                .iter()
                .filter(|(_, bot, _, live)| *live && *bot == request.bot_id)
                .map(|(id, ..)| AccessToken {
                    id: id.clone(),
                    name: "tokuly demo".into(),
                    scopes: setup::SCOPES.iter().rev().map(|scope| (*scope).to_owned()).collect(),
                    ..Default::default()
                })
                .collect();
            answer(ListBotTokensResponse { tokens, ..Default::default() })
        }
        "RevokeBotToken" => {
            let request: RevokeBotTokenRequest = read(&body);
            for token in server.tokens.iter_mut().filter(|token| token.0 == request.token_id) {
                token.3 = false;
            }
            answer(serde_json::json!({}))
        }
        "CreateBotToken" => {
            let request: CreateBotTokenRequest = read(&body);
            let created = server.created.entry(operator).or_default();
            *created += 1;
            if *created > TOKENS_AN_HOUR {
                return refusal(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");
            }
            let id = format!("token-{}", server.tokens.len());
            let secret = format!("tky_bot_{id}");
            server.tokens.push((id.clone(), request.bot_id, secret.clone(), true));
            let token = AccessToken { id, name: request.name, scopes: request.scopes, ..Default::default() };
            answer(CreateBotTokenResponse { token: token.into(), secret, ..Default::default() })
        }
        other => panic!("the stand-in does not know {other}"),
    }
}

async fn serve() -> (String, Shared) {
    let server = Shared::default();
    let app = Router::new().fallback(rpc).with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, server)
}

fn config(api_url: &str, state_dir: &Path) -> DemoConfig {
    DemoConfig {
        api_url: api_url.to_owned(),
        operators: vec!["a@demo.example".into(), "b@demo.example".into(), "c@demo.example".into()],
        password: "the operators' password".into(),
        seed: 2026,
        episode: Some(1),
        bots: 12,
        center: Center(LatLng { lat: 35.7030, lng: 139.5795 }),
        radius_m: 900.0,
        pace: 1.0,
        companions: 3,
        storage_url: None,
        state_dir: Some(state_dir.to_owned()),
        run_seconds: 0,
    }
}

#[tokio::test]
async fn the_demo_starts_again_and_again() {
    let (url, server) = serve().await;
    let state = tempfile::tempdir().unwrap();
    let config = config(&url, state.path());
    let script = Script::new(config.seed, 1, config.bots, config.region());
    let api = Api::new(&url, None).unwrap();

    let first = setup::prepare(&api, &config, &script).await.unwrap();
    // More starts than the allowance has room for if each made new tokens.
    for start in 2..=8 {
        let again =
            setup::prepare(&api, &config, &script).await.unwrap_or_else(|error| panic!("start {start}: {error:#}"));
        let tokens = |bots: &[setup::Bot]| bots.iter().map(|bot| bot.token.clone()).collect::<Vec<_>>();
        assert_eq!(tokens(&again), tokens(&first), "start {start} kept the tokens");
    }
    assert_eq!(server.lock().unwrap().created.values().sum::<usize>(), 12);

    // A token revoked by its owner (「開発者 → ボット」) is replaced, and only that one.
    let revoked = server.lock().unwrap().tokens.iter().position(|token| token.2 == first[3].token).unwrap();
    server.lock().unwrap().tokens[revoked].3 = false;
    let after = setup::prepare(&api, &config, &script).await.unwrap();
    assert_ne!(after[3].token, first[3].token);
    assert_eq!(after.iter().filter(|bot| first.iter().any(|old| old.token == bot.token)).count(), 11);
    assert_eq!(server.lock().unwrap().created.values().sum::<usize>(), 13);
    assert_eq!(server.lock().unwrap().tokens.iter().filter(|token| token.3).count(), 12);
}

#[tokio::test]
async fn without_the_kept_tokens_the_simulator_makes_new_ones_and_revokes_the_old() {
    let (url, server) = serve().await;
    let api = Api::new(&url, None).unwrap();
    let first_dir = tempfile::tempdir().unwrap();
    let config = config(&url, first_dir.path());
    let script = Script::new(config.seed, 1, config.bots, config.region());
    let first = setup::prepare(&api, &config, &script).await.unwrap();

    // The state was lost (a new volume).
    let second_dir = tempfile::tempdir().unwrap();
    let again = setup::prepare(&api, &DemoConfig { state_dir: Some(second_dir.path().to_owned()), ..config }, &script)
        .await
        .unwrap();
    assert!(again.iter().all(|bot| first.iter().all(|old| old.token != bot.token)));
    assert_eq!(server.lock().unwrap().tokens.iter().filter(|token| token.3).count(), 12);
}
