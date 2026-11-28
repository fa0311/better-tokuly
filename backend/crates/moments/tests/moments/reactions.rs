//! RealMoji and stamp reactions, the library, and reacting twice at once.
use tokuly_moments::KIND_REACTION;
use tokuly_proto::{
    media::AssetPurpose,
    moments::{
        ListReactionsRequest, ListReactionsResponse, ListRealmojisRequest, ListRealmojisResponse, Moment, ReactRequest,
        ReactResponse, RealmojiEmoji, RemoveReactionRequest, RemoveReactionResponse, SetRealmojiRequest,
        SetRealmojiResponse, moment_reaction::Reaction as Shown, react_request::Reaction,
    },
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

use crate::common::*;

/// Two friends in a region of their own, both posted; returns the second's moment.
async fn setup(ctx: &Ctx) -> (TestUser, TestUser, Moment) {
    let mut users = ctx.circle_of(&["あや", "べに"]).await;
    let (b, a) = (users.pop().unwrap(), users.pop().unwrap());
    let region = ctx.region(&[&a, &b]).await;
    ctx.prompt(&region).await;
    ctx.post(&a).await;
    let moment = ctx.post(&b).await;
    (a, b, moment)
}

async fn set_realmoji(ctx: &Ctx, user: &TestUser, emoji: RealmojiEmoji) -> String {
    let asset_id = ctx.picture(user, AssetPurpose::ASSET_PURPOSE_REALMOJI).await;
    let request = SetRealmojiRequest { emoji: emoji.into(), asset_id: asset_id.clone(), ..Default::default() };
    let _: SetRealmojiResponse = ctx.call("SetRealmoji", user, &request).await.unwrap();
    asset_id
}

async fn react(
    ctx: &Ctx,
    user: &TestUser,
    moment_id: &str,
    reaction: Reaction,
    asset_id: &str,
) -> Result<ReactResponse, RpcError> {
    let request = ReactRequest {
        moment_id: moment_id.to_owned(),
        reaction: Some(reaction),
        asset_id: asset_id.to_owned(),
        ..Default::default()
    };
    ctx.call("React", user, &request).await
}

fn face(emoji: RealmojiEmoji) -> Reaction {
    Reaction::Realmoji(emoji.into())
}

async fn reactions(ctx: &Ctx, user: &TestUser, moment_id: &str) -> Result<ListReactionsResponse, RpcError> {
    ctx.call("ListReactions", user, &ListReactionsRequest { moment_id: moment_id.to_owned(), ..Default::default() })
        .await
}

#[tokio::test]
async fn a_realmoji_keeps_the_picture_it_was_made_with() {
    let ctx = start().await;
    let (a, b, moment) = setup(&ctx).await;
    let smile = RealmojiEmoji::REALMOJI_EMOJI_SMILE;
    assert_eq!(react(&ctx, &a, &moment.id, face(smile), "").await.unwrap_err().reason, "NO_REALMOJI");

    let first = set_realmoji(&ctx, &a, smile).await;
    let reacted = react(&ctx, &a, &moment.id, face(smile), "").await.unwrap();
    let Some(Shown::Realmoji(made)) = &reacted.reaction.as_option().unwrap().reaction else { panic!("a RealMoji") };
    assert_eq!(made.image.asset_id, first);
    assert_eq!(ctx.notifications(&b, KIND_REACTION).await.len(), 1, "the owner is told");

    // A new selfie for the face changes the library, not what was sent.
    let second = set_realmoji(&ctx, &a, smile).await;
    let listed = reactions(&ctx, &b, &moment.id).await.unwrap();
    let Some(Shown::Realmoji(kept)) = &listed.reactions[0].reaction else { panic!("a RealMoji") };
    assert_eq!(kept.image.asset_id, first);
    assert_eq!(ctx.holders(&first).await, 1, "held by the reaction, no longer by the library");
    assert_eq!(ctx.holders(&second).await, 1);
    let library: ListRealmojisResponse = ctx.call("ListRealmojis", &a, &ListRealmojisRequest::default()).await.unwrap();
    assert_eq!(library.realmojis.len(), 1);
    assert_eq!(library.realmojis[0].image.asset_id, second);

    // ⚡ takes the selfie just taken; it replaces the reaction before.
    let instant = ctx.picture(&a, AssetPurpose::ASSET_PURPOSE_REALMOJI).await;
    react(&ctx, &a, &moment.id, face(RealmojiEmoji::REALMOJI_EMOJI_INSTANT), &instant).await.unwrap();
    let shown = ctx.get(&b, &moment.id).await.unwrap();
    assert_eq!(shown.reaction_count, 1, "one per person");
    let Some(Shown::Realmoji(now)) = &shown.reactions[0].reaction else { panic!("a RealMoji") };
    assert_eq!(
        (now.emoji, now.image.asset_id.as_str()),
        (RealmojiEmoji::REALMOJI_EMOJI_INSTANT.into(), instant.as_str())
    );
    assert_eq!(ctx.notifications(&b, KIND_REACTION).await.len(), 2);
    let set_instant = SetRealmojiRequest {
        emoji: RealmojiEmoji::REALMOJI_EMOJI_INSTANT.into(),
        asset_id: instant.clone(),
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, SetRealmojiResponse>("SetRealmoji", &a, &set_instant).await.unwrap_err().reason,
        "INVALID_REACTION"
    );

    let _: RemoveReactionResponse = ctx
        .call("RemoveReaction", &a, &RemoveReactionRequest { moment_id: moment.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(ctx.get(&b, &moment.id).await.unwrap().reaction_count, 0);
}

#[tokio::test]
async fn reacting_needs_an_unlocked_moment_of_someone_else() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに"]).await;
    let (a, b) = (&users[0], &users[1]);
    let stranger = ctx.app.user("しらない").await;
    let region = ctx.region(&[a, b, &stranger]).await;
    ctx.prompt(&region).await;
    let moment = ctx.post(b).await;
    let smile = RealmojiEmoji::REALMOJI_EMOJI_SMILE;
    set_realmoji(&ctx, a, smile).await;
    set_realmoji(&ctx, b, smile).await;

    assert_eq!(react(&ctx, a, &moment.id, face(smile), "").await.unwrap_err().reason, "MOMENT_LOCKED");
    assert_eq!(reactions(&ctx, a, &moment.id).await.unwrap_err().reason, "MOMENT_LOCKED");
    assert_eq!(react(&ctx, b, &moment.id, face(smile), "").await.unwrap_err().reason, "OWN_MOMENT");
    ctx.post(&stranger).await;
    assert_eq!(react(&ctx, &stranger, &moment.id, face(smile), "").await.unwrap_err().reason, "MOMENT_NOT_FOUND");

    ctx.post(a).await;
    let unknown = Reaction::StampId(Uuid::now_v7().to_string());
    assert_eq!(react(&ctx, a, &moment.id, unknown, "").await.unwrap_err().reason, "STAMP_NOT_USABLE");
    let nothing = ReactRequest { moment_id: moment.id.clone(), ..Default::default() };
    assert_eq!(ctx.call::<_, ReactResponse>("React", a, &nothing).await.unwrap_err().reason, "INVALID_REACTION");
    react(&ctx, a, &moment.id, face(smile), "").await.unwrap();

    // A block hides the reaction from the owner, and the moment from the reactor.
    ctx.block(b, a).await;
    assert_eq!(reactions(&ctx, b, &moment.id).await.unwrap().reactions.len(), 0);
    assert_eq!(ctx.get(b, &moment.id).await.unwrap().reaction_count, 0);
}

#[tokio::test]
async fn reacting_many_times_at_once_leaves_one_reaction() {
    let ctx = start().await;
    let (a, b, moment) = setup(&ctx).await;
    let faces = [
        RealmojiEmoji::REALMOJI_EMOJI_THUMBS_UP,
        RealmojiEmoji::REALMOJI_EMOJI_SMILE,
        RealmojiEmoji::REALMOJI_EMOJI_SURPRISED,
        RealmojiEmoji::REALMOJI_EMOJI_HEART_EYES,
        RealmojiEmoji::REALMOJI_EMOJI_LAUGHING,
    ];
    for emoji in faces {
        set_realmoji(&ctx, &a, emoji).await;
    }
    // As many at once as the test pool has connections.
    let results = futures::future::join_all(faces.map(|emoji| react(&ctx, &a, &moment.id, face(emoji), ""))).await;
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM moment_reactions WHERE moment_id = $1 AND user_id = $2 AND removed_at IS NULL",
    )
    .bind(id(&moment.id))
    .bind(a.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(live, 1);
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM moment_reactions WHERE moment_id = $1 AND user_id = $2")
        .bind(id(&moment.id))
        .bind(a.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(kept, 5, "the replaced ones stay on record");
    assert_eq!(reactions(&ctx, &b, &moment.id).await.unwrap().reactions.len(), 1);
}

#[tokio::test]
async fn selfies_set_at_once_leave_one_held() {
    let ctx = start().await;
    let a = ctx.app.user("あや").await;
    let mut assets = Vec::new();
    for _ in 0..5 {
        assets.push(ctx.picture(&a, AssetPurpose::ASSET_PURPOSE_REALMOJI).await);
    }
    let requests: Vec<SetRealmojiRequest> = assets
        .iter()
        .map(|asset_id| SetRealmojiRequest {
            emoji: RealmojiEmoji::REALMOJI_EMOJI_SMILE.into(),
            asset_id: asset_id.clone(),
            ..Default::default()
        })
        .collect();
    let results = futures::future::join_all(
        requests.iter().map(|request| ctx.call::<_, SetRealmojiResponse>("SetRealmoji", &a, request)),
    )
    .await;
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    let mut held = 0;
    for asset_id in &assets {
        held += ctx.holders(asset_id).await;
    }
    assert_eq!(held, 1, "only the selfie in the library is held");
}
