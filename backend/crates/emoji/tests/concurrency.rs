//! Requests that arrive at the same time.
//!
//! Every test here fires more calls at once than the test's database pool
//! has connections (five). Two things are checked each time: that what the
//! crate relies on — a lock, a unique index, the order of catalogue
//! versions — holds when the calls really overlap, and that nobody waits
//! for a connection somebody else can never give back (such a call would
//! fail with `INTERNAL` after the pool's ten seconds).
mod common;

use std::collections::{HashMap, HashSet};

use common::{pack::*, *};
use futures::future::join_all;
use tokuly_emoji::{EmojiConfig, catalog};
use tokuly_proto::emoji::*;
use tokuly_testkit::TestUser;

/// More than the pool's five connections.
const BURST: usize = 12;

/// How often each outcome occurred: `"ok"` or the reason of the refusal.
fn tally<T>(results: &[Result<T, tokuly_testkit::RpcError>]) -> HashMap<String, usize> {
    let mut outcomes: HashMap<String, usize> = HashMap::new();
    for result in results {
        let outcome = match result {
            Ok(_) => "ok".to_owned(),
            Err(error) => error.reason.clone(),
        };
        *outcomes.entry(outcome).or_default() += 1;
    }
    outcomes
}

fn outcomes(pairs: &[(&str, usize)]) -> HashMap<String, usize> {
    pairs.iter().map(|(outcome, count)| (outcome.to_string(), *count)).collect()
}

async fn pending_count(ctx: &Ctx, user: &TestUser) -> i32 {
    let response: GetSubmitterProfileResponse = ctx
        .call("Submission", "GetSubmitterProfile", Some(user), &GetSubmitterProfileRequest::default())
        .await
        .unwrap();
    response.profile.pending_count
}

#[tokio::test]
async fn one_account_cannot_stall_the_api_or_slip_past_its_limit_with_a_burst() {
    let ctx = start_with(EmojiConfig { max_pending: 3, ..Default::default() }).await;
    let (author, bystander) = (ctx.app.user("author").await, ctx.app.user("bystander").await);
    let mut requests = Vec::new();
    for n in 0..BURST {
        requests.push(ctx.submit_request(&author, &unique(&format!("burst{n}_"))).await);
    }

    // Twelve submissions of one account at once, while somebody else uses the API.
    let submissions = join_all(
        requests
            .iter()
            .map(|request| ctx.call::<_, SubmitEmojiResponse>("Submission", "SubmitEmoji", Some(&author), request)),
    );
    let (results, others_pending) = tokio::join!(submissions, pending_count(&ctx, &bystander));
    assert_eq!(others_pending, 0);
    assert_eq!(tally(&results), outcomes(&[("ok", 3), ("TOO_MANY_PENDING", BURST - 3)]), "the limit holds exactly");
    assert_eq!(pending_count(&ctx, &author).await, 3);

    // The same account changing one submission twelve times at once: the
    // changes queue up, every one is applied, none is lost to another's lock.
    let emoji_id = results.iter().find_map(|result| result.as_ref().ok()).unwrap().emoji.emoji.id.clone();
    let changes: Vec<UpdateSubmissionRequest> = (0..BURST)
        .map(|n| UpdateSubmissionRequest {
            emoji_id: emoji_id.clone(),
            license: Some(format!("license {n}")),
            ..Default::default()
        })
        .collect();
    let results = join_all(changes.iter().map(|change| {
        ctx.call::<_, UpdateSubmissionResponse>("Submission", "UpdateSubmission", Some(&author), change)
    }))
    .await;
    assert_eq!(tally(&results), outcomes(&[("ok", BURST)]));
    assert!(ctx.get(Some(&author), &emoji_id).await.unwrap().license.starts_with("license "));
    assert_eq!(pending_count(&ctx, &author).await, 3);
}

#[tokio::test]
async fn a_name_goes_to_exactly_one_of_many_simultaneous_submitters() {
    let ctx = start().await;
    let name = unique("wanted");
    let mut submitters = Vec::new();
    for n in 0..BURST {
        let user = ctx.app.user(&format!("submitter{n}")).await;
        let request = ctx.submit_request(&user, &name).await;
        submitters.push((user, request));
    }
    let results =
        join_all(submitters.iter().map(|(user, request)| {
            ctx.call::<_, SubmitEmojiResponse>("Submission", "SubmitEmoji", Some(user), request)
        }))
        .await;
    assert_eq!(tally(&results), outcomes(&[("ok", 1), ("NAME_TAKEN", BURST - 1)]));
    let holders: i64 = sqlx::query_scalar("SELECT count(*) FROM emojis WHERE name = $1")
        .bind(&name)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(holders, 1);
    // The uploads of the ones who lost are not kept in use.
    let held: i64 =
        sqlx::query_scalar("SELECT count(*) FROM media_assets WHERE id = ANY($1::uuid[]) AND holder_count > 0")
            .bind(submitters.iter().map(|(_, request)| request.asset_id.clone()).collect::<Vec<_>>())
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(held, 1);
}

#[tokio::test]
async fn a_decision_taken_many_times_at_once_happens_once() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let emoji = ctx.submit(&author, &unique("decided")).await;
    let id = emoji.emoji.id.clone();

    let approve = ApproveEmojiRequest { emoji_id: id.clone(), ..Default::default() };
    let results = join_all(
        (0..8).map(|_| ctx.call::<_, ApproveEmojiResponse>("Admin", "ApproveEmoji", Some(&moderator), &approve)),
    )
    .await;
    assert_eq!(tally(&results), outcomes(&[("ok", 1), ("EMOJI_NOT_REVIEWABLE", 7)]));
    assert_eq!(ctx.notification_kinds(&author).await, ["emoji.approved"], "the author is told once");
    assert_eq!(ctx.emoji_events("emoji.emoji.added", &id).await.len(), 1);
    assert_eq!(ctx.app.events_by("activity.recorded", author.id).await.len(), 1);
    assert_eq!(ctx.catalog().await.emojis.iter().filter(|listed| listed.id == id).count(), 1);

    let disable =
        DisableEmojiRequest { emoji_id: id.clone(), reason: "同時にきたよ".to_owned(), ..Default::default() };
    let results = join_all(
        (0..8).map(|_| ctx.call::<_, DisableEmojiResponse>("Admin", "DisableEmoji", Some(&moderator), &disable)),
    )
    .await;
    assert_eq!(tally(&results), outcomes(&[("ok", 1), ("EMOJI_NOT_APPROVED", 7)]));
    assert_eq!(ctx.notification_kinds(&author).await, ["emoji.approved", "emoji.disabled"]);
    assert_eq!(ctx.emoji_events("emoji.emoji.removed", &id).await.len(), 1);
    let history = ctx.get(Some(&moderator), &id).await.unwrap().history;
    assert_eq!(history.len(), 3, "submitted, approved, disabled — once each");

    // The trusted flag: set many times at once, announced once.
    let request = SetSubmitterTrustedRequest { user_id: author.id.to_string(), trusted: true, ..Default::default() };
    let results = join_all((0..6).map(|_| {
        ctx.call::<_, SetSubmitterTrustedResponse>("Admin", "SetSubmitterTrusted", Some(&moderator), &request)
    }))
    .await;
    assert_eq!(tally(&results), outcomes(&[("ok", 6)]));
    assert_eq!(ctx.notification_kinds(&author).await.iter().filter(|kind| *kind == "emoji.trusted").count(), 1);
    assert_eq!(ctx.app.events_by("emoji.submitter.trust_changed", moderator.id).await.len(), 1);
}

#[tokio::test]
async fn simultaneous_changes_each_get_a_catalogue_version_and_none_is_missed() {
    let ctx = start().await;
    let moderator = ctx.moderator().await;
    ctx.add(&moderator, &unique("warmup")).await;
    let synced = ctx.catalog().await.version;

    let mut requests = Vec::new();
    for n in 0..BURST {
        requests.push(ctx.add_request(&moderator, &unique(&format!("same_time{n}_"))).await);
    }
    let results = join_all(
        requests.iter().map(|request| ctx.call::<_, AddEmojiResponse>("Admin", "AddEmoji", Some(&moderator), request)),
    )
    .await;
    assert_eq!(tally(&results), outcomes(&[("ok", BURST)]));
    let ids: Vec<String> = results.iter().map(|result| result.as_ref().unwrap().emoji.emoji.id.clone()).collect();

    // Every change took a version of its own, after the client's.
    let versions: Vec<i64> = sqlx::query_scalar("SELECT catalog_version FROM emojis WHERE id = ANY($1::uuid[])")
        .bind(&ids)
        .fetch_all(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(versions.iter().collect::<HashSet<_>>().len(), BURST);
    assert!(versions.iter().all(|version| *version > synced));

    // A client that synced before sees all of them, and is up to date after.
    let caught_up = ctx.changes(synced).await;
    assert!(!caught_up.full);
    for id in &ids {
        assert!(caught_up.upserted.iter().any(|emoji| emoji.id == *id), "{id} is in the difference");
    }
    assert!(caught_up.version >= *versions.iter().max().unwrap());
    for request in &requests {
        assert!(catalog::exists(&ctx.app.core, &request.name).await.unwrap());
    }

    // Deleting them at once: each removal is reported, none twice.
    let (app, staff) = (&ctx, &moderator);
    let results = join_all(ids.iter().map(|id| {
        let request = DeleteEmojiRequest { emoji_id: id.clone(), ..Default::default() };
        async move { app.call::<_, DeleteEmojiResponse>("Admin", "DeleteEmoji", Some(staff), &request).await }
    }))
    .await;
    assert_eq!(tally(&results), outcomes(&[("ok", BURST)]));
    let gone = ctx.changes(caught_up.version).await;
    let removed: HashSet<&String> = gone.removed_ids.iter().collect();
    assert_eq!(gone.removed_ids.len(), removed.len());
    assert!(ids.iter().all(|id| removed.contains(id)));
}

#[tokio::test]
async fn the_pictures_of_an_import_arriving_together_all_become_emoji() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let moderator = ctx.moderator().await;
    let t = unique("");
    let names: Vec<String> = (0..BURST).map(|n| format!("pic{n}_{t}")).collect();
    let files: Vec<String> = (0..BURST).map(|n| format!("{n}.png")).collect();
    let records = names.iter().zip(&files).map(|(name, file)| record(name, file, serde_json::json!({}))).collect();
    let picture = png(40);
    let mut archive: Vec<(&str, &[u8])> = files.iter().map(|file| (file.as_str(), picture.as_slice())).collect();
    let meta = meta(records);
    archive.push(("meta.json", &meta));
    let archive = upload_archive(&ctx, &moderator, zip_of(&archive)).await;

    let import = start_import(&ctx, &moderator, &archive, ImportConflictPolicy::IMPORT_CONFLICT_POLICY_SKIP, "").await;
    run_import(&ctx, &import.id).await;
    let uploads = process_pictures(&ctx, &import.id).await;
    assert_eq!(uploads.len(), BURST);

    // Media finished them all; its events are handled at the same time, as
    // the consumer does (eight at once) — and one of them twice.
    let mut events: Vec<_> = ctx
        .app
        .events("media.asset.ready")
        .await
        .into_iter()
        .filter(|event| {
            event.data["asset_id"].as_str().and_then(|id| id.parse().ok()).is_some_and(|id| uploads.contains(&id))
        })
        .collect();
    assert_eq!(events.len(), BURST);
    events.push(events[0].clone());
    let handled = join_all(events.into_iter().map(|event| ctx.emoji.handle_event(event))).await;
    assert!(handled.iter().all(Result::is_ok), "{handled:?}");

    let done = get_import(&ctx, &moderator, &import.id).await;
    assert_eq!(done.import.status, ImportStatus::IMPORT_STATUS_COMPLETED);
    assert_eq!((done.import.imported_count, done.import.skipped_count, done.import.failed_count), (BURST as i32, 0, 0));
    assert!(done.entries.iter().all(|entry| entry.status == ImportEntryStatus::IMPORT_ENTRY_STATUS_IMPORTED));
    let catalogue = ctx.catalog().await;
    for name in &names {
        assert!(named(&catalogue.emojis, name).is_some(), "{name} is in the catalogue");
    }
    assert_eq!(ctx.holders(&archive).await, 0);
}

/// The only test of this file that runs the usage worker (its counters are
/// one set for the whole database).
#[tokio::test]
async fn use_counters_written_by_several_workers_at_once_are_written_once() {
    let ctx = start().await;
    let moderator = ctx.moderator().await;
    let name = unique("counted");
    ctx.add(&moderator, &name).await;
    let mut fans = Vec::new();
    for n in 0..BURST {
        fans.push(ctx.app.user(&format!("fan{n}")).await);
    }
    // Everyone uses it at once…
    let app = &ctx;
    let recorded = join_all(fans.iter().map(|fan| {
        let request = RecordUseRequest { emojis: vec![format!(":{name}:"), "👍".to_owned()], ..Default::default() };
        async move { app.call::<_, RecordUseResponse>("", "RecordUse", Some(fan), &request).await }
    }))
    .await;
    assert_eq!(tally(&recorded), outcomes(&[("ok", BURST)]));
    // …and several workers pick the counters up at once.
    let written = join_all((0..6).map(|_| ctx.emoji.aggregate_usage())).await;
    let total: u64 = written.into_iter().map(|written| written.expect("usage aggregation")).sum();
    assert_eq!(total, BURST as u64);
    assert_eq!(ctx.get(Some(&moderator), &name).await.unwrap().use_count, BURST as i64);
    let today: (i64, i64) = sqlx::query_as(
        "SELECT uses, users FROM emoji_usage_daily d JOIN emojis e ON e.id = d.emoji_id WHERE e.name = $1",
    )
    .bind(&name)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(today, (BURST as i64, BURST as i64), "each of them used it once");
    assert_eq!(ctx.emoji.aggregate_usage().await.unwrap(), 0);
}
