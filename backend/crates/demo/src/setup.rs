//! Getting the cast onto the server: operator accounts, their bots, a token
//! per bot, profiles, and the friendships between the bots.
//!
//! Everything here can run again: a second start finds the accounts, the
//! bots and the tokens of the first ([`Keep`]) and only fills in what is
//! missing.
use anyhow::{Context as _, bail};
use tokuly_proto::{
    dev::{
        Bot as BotAccount, CreateBotRequest, CreateBotResponse, CreateBotTokenRequest, CreateBotTokenResponse,
        ListBotTokensRequest, ListBotTokensResponse, ListBotsRequest, ListBotsResponse, RevokeBotTokenRequest,
        RevokeBotTokenResponse, TokenExpiry, UpdateBotRequest, UpdateBotResponse,
    },
    identity::{
        AuthResponse, ClientInfo, SignInRequest, SignOutRequest, SignOutResponse, SignUpRequest, UpdateProfileRequest,
        UpdateProfileResponse,
    },
    media::AssetPurpose,
    notes::{AgreeToNoteRulesRequest, AgreeToNoteRulesResponse, GetNoteRulesRequest, GetNoteRulesResponse},
    social::{SendFriendRequestRequest, SendFriendRequestResponse, send_friend_request_request::Target},
};

use crate::{
    api::{Api, ApiError},
    cast::Persona,
    config::DemoConfig,
    keep::{Keep, KeptToken},
    media::{self, Upload},
    script::Script,
};

/// The name of the simulator's tokens. A start revokes those it did not keep.
const TOKEN_NAME: &str = "tokuly demo";

/// What a bot's token may do: what the bots do, and nothing else. (No
/// wallet scope exists for bots at all.)
pub const SCOPES: &[&str] = &[
    "identity:write",
    "media:write",
    "geo:write",
    "social:write",
    "chat:write",
    "notes:write",
    "video:write",
    "shorts:write",
    "engagement:write",
];

/// A bot of the cast as an account.
#[derive(Debug, Clone)]
pub struct Bot {
    pub id: String,
    pub name: String,
    /// The API token everything the bot does is done with.
    pub token: String,
}

/// Brings the cast of `script` onto the server and returns the bots in
/// cast order.
pub async fn prepare(api: &Api, config: &DemoConfig, script: &Script) -> anyhow::Result<Vec<Bot>> {
    let cast = script.cast();
    let mut keep = Keep::open(config.state_dir().as_deref(), &config.password);
    let mut bots = Vec::with_capacity(cast.len());
    for (number, address) in config.operators().into_iter().enumerate() {
        if bots.len() == cast.len() {
            break;
        }
        let session =
            sign_in(api, address, &config.password, number).await.with_context(|| format!("operator {address}"))?;
        let owned: ListBotsResponse =
            api.call(Some(&session), "tokuly.dev.v1.BotService/ListBots", &ListBotsRequest::default()).await?;
        let room = (owned.max_bots.max(0) as usize).max(owned.bots.len());
        for slot in 0..room.min(cast.len() - bots.len()) {
            let persona = &cast[bots.len()];
            let account = match owned.bots.get(slot) {
                Some(existing) => refreshed(api, &session, existing, persona).await,
                None => create(api, &session, persona).await.with_context(|| format!("bot {}", persona.handle))?,
            };
            let token = token(api, &session, &account, &mut keep)
                .await
                .with_context(|| format!("token of {}", persona.handle))?;
            bots.push(Bot { id: account, name: persona.display_name.clone(), token });
        }
        // The session was only needed for this.
        let _ = api
            .call::<_, SignOutResponse>(
                Some(&session),
                "tokuly.identity.v1.AuthService/SignOut",
                &SignOutRequest::default(),
            )
            .await;
    }
    if bots.len() < cast.len() {
        bail!(
            "{} bots do not fit {} operator account(s) ({} fit): add addresses to TOKULY_DEV_DEMO_OPERATORS or lower TOKULY_DEMO_BOTS",
            cast.len(),
            config.operators().len(),
            bots.len()
        );
    }
    let profiles = bots.iter().zip(cast).map(|(bot, persona)| profile(api, bot, persona));
    for (result, persona) in futures::future::join_all(profiles).await.into_iter().zip(cast) {
        result.with_context(|| format!("profile of {}", persona.handle))?;
    }
    for &(a, b) in script.friendships() {
        // Requests that cross make friends at once.
        for (from, to) in [(&bots[a], &bots[b]), (&bots[b], &bots[a])] {
            befriend(api, from, &to.id, "")
                .await
                .or_else(|refused| if refused.is("ALREADY_FRIENDS") { Ok(()) } else { Err(refused) })?;
        }
    }
    Ok(bots)
}

/// Signs the operator in, or up on the first run. Returns its access token.
async fn sign_in(api: &Api, address: &str, password: &str, number: usize) -> anyhow::Result<String> {
    let client = ClientInfo {
        device_name: "tokuly demo".into(),
        device_id: format!("tokuly-demo-{number}"),
        locale: "ja".into(),
        time_zone: "Asia/Tokyo".into(),
        ..Default::default()
    };
    let request = SignInRequest {
        identifier: address.to_owned(),
        password: password.to_owned(),
        client: client.clone().into(),
        ..Default::default()
    };
    let session = match api.call::<_, AuthResponse>(None, "tokuly.identity.v1.AuthService/SignIn", &request).await {
        Ok(session) => session,
        Err(refused) if refused.is("INVALID_CREDENTIALS") => {
            let request = SignUpRequest {
                email: address.to_owned(),
                password: password.to_owned(),
                handle: format!("demo_staff_{}", number + 1),
                display_name: "デモ運営".into(),
                // Nobody's birthday: the account is not a person.
                birthdate: "2000-01-01".into(),
                client: client.into(),
                accept_terms: true,
                ..Default::default()
            };
            match api.call(None, "tokuly.identity.v1.AuthService/SignUp", &request).await {
                Ok(session) => session,
                Err(taken) if taken.is("EMAIL_TAKEN") || taken.is("ACCOUNT_EXISTS") => {
                    bail!("an account with this address exists and TOKULY_DEV_DEMO_PASSWORD is not its password")
                }
                Err(refused) => return Err(refused.into()),
            }
        }
        Err(refused) => return Err(refused.into()),
    };
    anyhow::ensure!(
        !session.mfa_required && !session.access_token.is_empty(),
        "the account asks for a second factor; the simulator cannot give one"
    );
    Ok(session.access_token)
}

/// Creates the bot account of `persona`. Returns its ID.
async fn create(api: &Api, session: &str, persona: &Persona) -> Result<String, ApiError> {
    // Someone may hold the handle already: then the bot gets a numbered one.
    let handles = std::iter::once(persona.handle.clone()).chain((2..=9).map(|n| format!("{}_{n}", persona.handle)));
    let mut refused = None;
    for handle in handles {
        let request = CreateBotRequest {
            handle,
            display_name: persona.display_name.clone(),
            bio: persona.bio.clone(),
            ..Default::default()
        };
        match api.call::<_, CreateBotResponse>(Some(session), "tokuly.dev.v1.BotService/CreateBot", &request).await {
            Ok(created) => return Ok(account_id(created.bot.as_option())),
            Err(taken) if taken.is("HANDLE_TAKEN") => refused = Some(taken),
            Err(other) => return Err(other),
        }
    }
    Err(refused.expect("the loop ran"))
}

/// An existing bot, renamed when the cast changed (another seed). Returns its ID.
async fn refreshed(api: &Api, session: &str, existing: &BotAccount, persona: &Persona) -> String {
    let id = account_id(Some(existing));
    let name = existing.user.as_option().map(|user| user.display_name.as_str());
    if name != Some(&persona.display_name) || existing.bio != persona.bio {
        let request = UpdateBotRequest {
            bot_id: id.clone(),
            display_name: Some(persona.display_name.clone()),
            bio: Some(persona.bio.clone()),
            ..Default::default()
        };
        if let Err(refused) =
            api.call::<_, UpdateBotResponse>(Some(session), "tokuly.dev.v1.BotService/UpdateBot", &request).await
        {
            tracing::warn!(bot = %persona.handle, %refused, "could not rename the bot; it keeps its profile");
        }
    }
    id
}

fn account_id(bot: Option<&BotAccount>) -> String {
    bot.and_then(|bot| bot.user.as_option()).map(|user| user.id.clone()).unwrap_or_default()
}

/// The bot's token: the one the last start kept while the server still has
/// it with the scopes of [`SCOPES`], else a new one. The simulator's other
/// tokens of the bot are revoked.
async fn token(api: &Api, session: &str, bot_id: &str, keep: &mut Keep) -> anyhow::Result<String> {
    let session = Some(session);
    let request = ListBotTokensRequest { bot_id: bot_id.to_owned(), ..Default::default() };
    let live: ListBotTokensResponse = api.call(session, "tokuly.dev.v1.BotService/ListBotTokens", &request).await?;
    let wanted = |scopes: &[String]| {
        let mut scopes: Vec<&str> = scopes.iter().map(String::as_str).collect();
        scopes.sort_unstable();
        let mut wanted = SCOPES.to_vec();
        wanted.sort_unstable();
        scopes == wanted
    };
    let kept = keep
        .get(bot_id)
        .filter(|kept| live.tokens.iter().any(|token| token.id == kept.id && !token.expired && wanted(&token.scopes)))
        .cloned();
    for old in live
        .tokens
        .iter()
        .filter(|token| token.name == TOKEN_NAME && kept.as_ref().is_none_or(|kept| kept.id != token.id))
    {
        let request = RevokeBotTokenRequest { token_id: old.id.clone(), ..Default::default() };
        api.call::<_, RevokeBotTokenResponse>(session, "tokuly.dev.v1.BotService/RevokeBotToken", &request).await?;
    }
    if let Some(kept) = kept {
        return Ok(kept.secret);
    }
    let request = CreateBotTokenRequest {
        bot_id: bot_id.to_owned(),
        name: TOKEN_NAME.to_owned(),
        scopes: SCOPES.iter().map(|scope| (*scope).to_owned()).collect(),
        // Kept for the next start; the simulator may run for months.
        expiry: TokenExpiry::TOKEN_EXPIRY_NEVER.into(),
        ..Default::default()
    };
    let created = match api
        .call::<_, CreateBotTokenResponse>(session, "tokuly.dev.v1.BotService/CreateBotToken", &request)
        .await
    {
        Ok(created) => created,
        Err(refused) if refused.is("SCOPE_NOT_FOR_BOTS") => bail!(
            "the server does not let these bots move, chat and post: give the server (api and worker) the same TOKULY_DEV_DEMO_OPERATORS as the simulator and restart it"
        ),
        Err(refused) => return Err(refused.into()),
    };
    let id = created.token.as_option().map(|token| token.id.clone()).unwrap_or_default();
    // The bot works without it; only the next start makes a token again.
    if let Err(error) = keep.put(bot_id, KeptToken { id, secret: created.secret.clone() }) {
        tracing::warn!(%error, "demo: the bot token is not kept");
    }
    Ok(created.secret)
}

/// What a bot does once for itself: a face, and the note rules.
async fn profile(api: &Api, bot: &Bot, persona: &Persona) -> anyhow::Result<()> {
    let token = Some(bot.token.as_str());
    let me: tokuly_proto::identity::GetMeResponse = api
        .call(token, "tokuly.identity.v1.AccountService/GetMe", &tokuly_proto::identity::GetMeRequest::default())
        .await?;
    let has_avatar = me.me.as_option().and_then(|me| me.user.as_option()).is_some_and(|user| user.avatar.is_set());
    if !has_avatar {
        let picture = Upload {
            purpose: AssetPurpose::ASSET_PURPOSE_AVATAR,
            file_name: "avatar.png",
            content_type: "image/png",
            bytes: media::paint(persona.colours, 256, 256),
        };
        // A bot without a face still works.
        match media::upload(api, &bot.token, picture).await {
            Ok(asset) => {
                let request = UpdateProfileRequest { avatar_asset_id: Some(asset), ..Default::default() };
                api.call::<_, UpdateProfileResponse>(
                    token,
                    "tokuly.identity.v1.AccountService/UpdateProfile",
                    &request,
                )
                .await?;
            }
            Err(refused) => tracing::warn!(bot = %persona.handle, %refused, "no avatar"),
        }
    }
    let rules: GetNoteRulesResponse =
        api.call(token, "tokuly.notes.v1.NoteService/GetNoteRules", &GetNoteRulesRequest::default()).await?;
    if !rules.agreed {
        let request = AgreeToNoteRulesRequest { version: rules.version, ..Default::default() };
        api.call::<_, AgreeToNoteRulesResponse>(token, "tokuly.notes.v1.NoteService/AgreeToNoteRules", &request)
            .await?;
    }
    Ok(())
}

/// Sends a friend request as `from` (accepting the one `to` sent, if any).
pub async fn befriend(api: &Api, from: &Bot, to: &str, message: &str) -> Result<(), ApiError> {
    let request = SendFriendRequestRequest {
        target: Some(Target::UserId(to.to_owned())),
        message: message.to_owned(),
        ..Default::default()
    };
    api.call::<_, SendFriendRequestResponse>(
        Some(&from.token),
        "tokuly.social.v1.FriendService/SendFriendRequest",
        &request,
    )
    .await
    .map(drop)
}
