//! Defaults by age group, erased accounts, moderation, reports after a
//! deletion, the purge and uploads that are gone.
use serde_json::json;
use tokuly_proto::{
    common::SubjectType,
    media::AssetPurpose,
    moments::{
        DeleteMomentRequest, DeleteMomentResponse, GetMomentSettingsRequest, GetMomentSettingsResponse, ReactRequest,
        ReactResponse, RealmojiEmoji, RemoveReactionRequest, RemoveReactionResponse, UpdateMomentSettingsRequest,
        UpdateMomentSettingsResponse, react_request::Reaction,
    },
};
use tokuly_testkit::TestUser;

use crate::common::*;

async fn share_location(ctx: &Ctx, user: &TestUser) -> bool {
    let got: GetMomentSettingsResponse =
        ctx.call("GetMomentSettings", user, &GetMomentSettingsRequest::default()).await.unwrap();
    got.settings.as_option().unwrap().share_location
}

#[tokio::test]
async fn the_location_default_follows_the_age_group_until_chosen() {
    let ctx = start().await;
    let (adult, minor, chooser, unknown) = (
        ctx.app.user("おとな").await,
        ctx.app.user("こども").await,
        ctx.app.user("えらぶ").await,
        ctx.app.user("なぞ").await,
    );
    assert!(!share_location(&ctx, &unknown).await, "no row reads as a minor's defaults");

    ctx.deliver("identity.user.created", json!({ "user_id": adult.id, "minor": false })).await;
    ctx.deliver("identity.user.created", json!({ "user_id": minor.id, "minor": true })).await;
    // Delivered twice, the same.
    ctx.deliver("identity.user.created", json!({ "user_id": adult.id, "minor": false })).await;
    assert!(share_location(&ctx, &adult).await);
    assert!(!share_location(&ctx, &minor).await);

    // An older account gives its birthdate: the defaults of its group, once.
    ctx.deliver("identity.user.birthdate_given", json!({ "user_id": unknown.id, "minor": false })).await;
    assert!(share_location(&ctx, &unknown).await);
    ctx.deliver("identity.user.created", json!({ "user_id": unknown.id })).await;
    assert!(!share_location(&ctx, &unknown).await, "an event without the group counts as a minor's");

    // Once chosen, no age default changes it.
    let choose = UpdateMomentSettingsRequest { share_location: Some(true), ..Default::default() };
    let _: UpdateMomentSettingsResponse = ctx.call("UpdateMomentSettings", &chooser, &choose).await.unwrap();
    ctx.deliver("identity.user.birthdate_given", json!({ "user_id": chooser.id, "minor": true })).await;
    assert!(share_location(&ctx, &chooser).await);
    let unknown_audience = json!({ "audience": 7 });
    assert_eq!(
        ctx.call::<_, UpdateMomentSettingsResponse>("UpdateMomentSettings", &chooser, &unknown_audience)
            .await
            .unwrap_err()
            .reason,
        "INVALID_AUDIENCE"
    );
}

#[tokio::test]
async fn an_erased_account_leaves_nothing_behind() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに"]).await;
    let (a, b) = (&users[0], &users[1]);
    let region = ctx.region(&[a, b]).await;
    ctx.prompt(&region).await;
    let theirs = ctx.post(b).await;
    let mine = ctx.post(a).await;
    let selfie = ctx.picture(a, AssetPurpose::ASSET_PURPOSE_REALMOJI).await;
    let request = ReactRequest {
        moment_id: theirs.id.clone(),
        reaction: Some(Reaction::Realmoji(RealmojiEmoji::REALMOJI_EMOJI_INSTANT.into())),
        asset_id: selfie.clone(),
        ..Default::default()
    };
    let reacted: ReactResponse = ctx.call("React", a, &request).await.unwrap();
    let reaction_id = reacted.reaction.as_option().unwrap().id.clone();

    ctx.deliver("identity.user.deleted", json!({ "user_id": a.id })).await;
    ctx.deliver("identity.user.deleted", json!({ "user_id": a.id })).await;
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM moments WHERE user_id = $1) + (SELECT count(*) FROM moment_reactions WHERE user_id = $1)
              + (SELECT count(*) FROM moment_deliveries WHERE user_id = $1)",
    )
    .bind(a.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(left, 0);
    assert_eq!(ctx.holders(&mine.back.asset_id).await, 0, "uploads released");
    assert_eq!(ctx.holders(&selfie).await, 0);
    assert!(ctx.app.subject_deleted(SubjectType::SUBJECT_TYPE_MOMENT, &mine.id).await);
    assert!(ctx.app.subject_deleted(SubjectType::SUBJECT_TYPE_MOMENT_REACTION, &reaction_id).await);
    let deleted: Vec<_> = ctx
        .app
        .events("moments.moment.deleted")
        .await
        .into_iter()
        .filter(|event| event.data["moment_id"] == mine.id)
        .collect();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].data["reason"], "account_deleted");
}

#[tokio::test]
async fn what_was_seen_stays_reportable_until_the_purge() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに", "ちか"]).await;
    let (owner, seen, unseen) = (&users[0], &users[1], &users[2]);
    let region = ctx.region(&[owner, seen, unseen]).await;
    ctx.prompt(&region).await;
    ctx.post(seen).await;
    let moment = ctx.post(owner).await;
    // `seen` opens it unlocked; `unseen` never posts, so never sees it.
    assert!(!ctx.get(seen, &moment.id).await.unwrap().locked);
    assert!(ctx.get(unseen, &moment.id).await.unwrap().locked);

    let subjects = &ctx.app.core.subjects;
    let reportable = |viewer: &TestUser| {
        let moment_id = moment.id.clone();
        let viewer = viewer.id;
        async move { subjects.resolve_for_report(SubjectType::SUBJECT_TYPE_MOMENT, &moment_id, Some(viewer)).await.is_ok() }
    };
    let _: DeleteMomentResponse = ctx
        .call("DeleteMoment", owner, &DeleteMomentRequest { moment_id: moment.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(subjects.resolve(SubjectType::SUBJECT_TYPE_MOMENT, &moment.id, Some(seen.id)).await.is_err());
    assert!(reportable(seen).await, "shown to them, so still reportable");
    assert!(!reportable(unseen).await, "a blur is nothing to report");
    assert!(reportable(owner).await);

    // The evidence period is over: gone for good, and announced.
    sqlx::query("UPDATE moments SET deleted_at = now() - interval '31 days' WHERE id = $1")
        .bind(id(&moment.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let (moments_gone, _) = ctx.moments.purge().await.unwrap();
    assert!(moments_gone >= 1);
    assert!(!reportable(seen).await);
    assert!(ctx.app.subject_deleted(SubjectType::SUBJECT_TYPE_MOMENT, &moment.id).await);
    assert_eq!(ctx.holders(&moment.back.asset_id).await, 0);
}

#[tokio::test]
async fn a_taken_back_reaction_can_still_be_reported_by_the_owner() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに"]).await;
    let (owner, reactor) = (&users[0], &users[1]);
    let region = ctx.region(&[owner, reactor]).await;
    ctx.prompt(&region).await;
    ctx.post(reactor).await;
    let moment = ctx.post(owner).await;
    let selfie = ctx.picture(reactor, AssetPurpose::ASSET_PURPOSE_REALMOJI).await;
    let request = ReactRequest {
        moment_id: moment.id.clone(),
        reaction: Some(Reaction::Realmoji(RealmojiEmoji::REALMOJI_EMOJI_INSTANT.into())),
        asset_id: selfie,
        ..Default::default()
    };
    let reacted: ReactResponse = ctx.call("React", reactor, &request).await.unwrap();
    let reaction_id = reacted.reaction.as_option().unwrap().id.clone();
    let _: RemoveReactionResponse = ctx
        .call("RemoveReaction", reactor, &RemoveReactionRequest { moment_id: moment.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let subjects = &ctx.app.core.subjects;
    let kind = SubjectType::SUBJECT_TYPE_MOMENT_REACTION;
    assert!(subjects.resolve(kind, &reaction_id, Some(owner.id)).await.is_err());
    let report = subjects.resolve_for_report(kind, &reaction_id, Some(owner.id)).await.unwrap();
    assert_eq!(report.owner_id, reactor.id, "the report is about the reactor");
    let outsider = ctx.app.user("そと").await;
    assert!(subjects.resolve_for_report(kind, &reaction_id, Some(outsider.id)).await.is_err());

    // Moderation takes the moment down; it stays on record like a deletion.
    ctx.deliver(
        "admin.content.removed",
        json!({ "subject_type": SubjectType::SUBJECT_TYPE_MOMENT as i32, "subject_id": moment.id, "moderator_id": null }),
    )
    .await;
    let reason: Option<String> = sqlx::query_scalar("SELECT delete_reason FROM moments WHERE id = $1")
        .bind(id(&moment.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(reason.as_deref(), Some("moderation"));
    assert_eq!(ctx.get(reactor, &moment.id).await.unwrap_err().reason, "MOMENT_NOT_FOUND");
}

#[tokio::test]
async fn a_moment_whose_upload_is_gone_is_deleted() {
    let ctx = start().await;
    let a = ctx.app.user("あや").await;
    let region = ctx.region(&[&a]).await;
    ctx.prompt(&region).await;
    let moment = ctx.post(&a).await;
    ctx.deliver("media.asset.deleted", json!({ "asset_id": moment.front.asset_id })).await;
    let reason: Option<String> = sqlx::query_scalar("SELECT delete_reason FROM moments WHERE id = $1")
        .bind(id(&moment.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(reason.as_deref(), Some("media_deleted"));
    // Unknown or repeated news changes nothing.
    ctx.deliver("media.asset.deleted", json!({ "asset_id": moment.front.asset_id })).await;
    ctx.deliver(
        "admin.content.removed",
        json!({ "subject_type": SubjectType::SUBJECT_TYPE_MOMENT as i32, "subject_id": uuid::Uuid::now_v7() }),
    )
    .await;
}
