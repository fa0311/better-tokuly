//! The プロフ帳, bots and birthday sharing, against the real infrastructure.
mod common;

use common::{Ext, start, unique};
use tokuly_identity::IdentityConfig;
use tokuly_proto::{common::Visibility, identity::*, media::AssetPurpose};
use tokuly_testkit::TestUser;

const UPDATE: &str = "tokuly.identity.v1.AccountService/UpdateProfileExtras";
const GET_PROFILE: &str = "tokuly.identity.v1.ProfileService/GetProfile";

fn vis(v: Visibility) -> buffa::EnumValue<Visibility> {
    v.into()
}

async fn profile(ext: &Ext, viewer: Option<&TestUser>, owner: &TestUser) -> Result<Profile, tokuly_testkit::RpcError> {
    let request =
        GetProfileRequest { by: Some(get_profile_request::By::UserId(owner.id.to_string())), ..Default::default() };
    let response: GetProfileResponse = ext.app.call(GET_PROFILE, viewer, &request).await?;
    Ok(response.profile.into_option().unwrap())
}

async fn update(
    ext: &Ext,
    user: &TestUser,
    request: &UpdateProfileExtrasRequest,
) -> Result<ProfileExtras, tokuly_testkit::RpcError> {
    let response: UpdateProfileExtrasResponse = ext.app.call(UPDATE, Some(user), request).await?;
    Ok(response.extras.into_option().unwrap())
}

fn card(prompt: &str, question: &str, answer: &str, visibility: Visibility, hidden: bool) -> ProfileQaCardInput {
    ProfileQaCardInput {
        prompt_id: prompt.into(),
        question: question.into(),
        answer: answer.into(),
        visibility: vis(visibility),
        hidden,
        ..Default::default()
    }
}

fn tag(category: ProfileTagCategory, name: &str) -> ProfileTag {
    ProfileTag { category: category.into(), name: name.into(), ..Default::default() }
}

fn everything_public() -> ProfileFieldVisibility {
    let p = vis(Visibility::VISIBILITY_PUBLIC);
    ProfileFieldVisibility {
        mbti: p,
        blood_type: p,
        pronouns: p,
        zodiac: p,
        oshi: p,
        tags: p,
        links: p,
        long_bio: p,
        pinned_song: p,
        ..Default::default()
    }
}

#[tokio::test]
async fn each_field_and_card_reaches_its_own_audience() {
    let ext = start(IdentityConfig::default()).await;
    let owner = ext.app.user("Yui").await;
    let friend = ext.app.user("Mio").await;
    let stranger = ext.app.user("Ken").await;
    ext.social.befriend(owner.id, friend.id);

    let mut visibility = everything_public();
    visibility.mbti = vis(Visibility::VISIBILITY_FRIENDS);
    visibility.pronouns = vis(Visibility::VISIBILITY_PRIVATE);
    let saved = update(
        &ext,
        &owner,
        &UpdateProfileExtrasRequest {
            mbti: Some("enfp-t".into()),
            blood_type: Some(BloodType::BLOOD_TYPE_O.into()),
            pronouns: Some("she/her".into()),
            long_bio: Some("ヨルシカと猫が好き\n\nよろしくね!".into()),
            theme: ProfileTheme {
                color: "#FF66AA".into(),
                style: ProfileThemeStyle::PROFILE_THEME_STYLE_PASTEL.into(),
                ..Default::default()
            }
            .into(),
            visibility: visibility.into(),
            tags: ProfileTagList {
                items: vec![
                    tag(ProfileTagCategory::PROFILE_TAG_CATEGORY_MUSIC, "ヨルシカ"),
                    tag(ProfileTagCategory::PROFILE_TAG_CATEGORY_FOOD, "ラーメン"),
                    // The same thing in another width is the same tag.
                    tag(ProfileTagCategory::PROFILE_TAG_CATEGORY_MUSIC, "ﾖﾙｼｶ"),
                ],
                ..Default::default()
            }
            .into(),
            qa_cards: ProfileQaCardList {
                items: vec![
                    card("fav_subject", "", "数学", Visibility::VISIBILITY_PUBLIC, false),
                    card("", "推しカプは?", "ひみつ", Visibility::VISIBILITY_FRIENDS, false),
                    card("dream", "", "パティシエ", Visibility::VISIBILITY_PRIVATE, false),
                    card("desert_island", "", "スマホ", Visibility::VISIBILITY_PUBLIC, true),
                ],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.mbti, "ENFP-T");
    assert_eq!(saved.tags.len(), 2);
    assert_eq!(saved.qa_cards.len(), 4, "the owner sees every card, hidden ones included");
    assert_eq!(saved.theme.as_option().unwrap().color, "#ff66aa");
    assert_eq!(saved.visibility.as_option().unwrap().pronouns, Visibility::VISIBILITY_PRIVATE);

    let seen = profile(&ext, Some(&stranger), &owner).await.unwrap();
    let extras = seen.extras.as_option().unwrap();
    assert_eq!(extras.mbti, "", "friends only");
    assert_eq!(extras.pronouns, "", "only me");
    assert_eq!(extras.blood_type, BloodType::BLOOD_TYPE_O);
    assert_eq!(extras.long_bio, "ヨルシカと猫が好き\n\nよろしくね!");
    assert_eq!(extras.qa_cards.iter().map(|c| c.answer.as_str()).collect::<Vec<_>>(), ["数学"]);
    assert_eq!(extras.qa_cards[0].question.as_option().unwrap().ja, "好きな教科");
    assert!(extras.visibility.as_option().is_none(), "audiences are the owner's business");
    assert_eq!(extras.qa_cards[0].visibility, Visibility::VISIBILITY_UNSPECIFIED);
    let anonymous = profile(&ext, None, &owner).await.unwrap();
    assert_eq!(anonymous.extras.as_option().unwrap().qa_cards.len(), 1);

    let seen = profile(&ext, Some(&friend), &owner).await.unwrap();
    let extras = seen.extras.as_option().unwrap();
    assert_eq!(extras.mbti, "ENFP-T");
    assert_eq!(extras.pronouns, "");
    assert_eq!(extras.qa_cards.iter().map(|c| c.answer.as_str()).collect::<Vec<_>>(), ["数学", "ひみつ"]);
    assert_eq!(extras.qa_cards[1].question.as_option().unwrap().ja, "推しカプは?");

    // A block, either way, hides the whole profile.
    ext.social.block(owner.id, stranger.id);
    assert_eq!(profile(&ext, Some(&stranger), &owner).await.unwrap_err().reason, "USER_NOT_FOUND");
    ext.social.block(friend.id, owner.id);
    assert_eq!(profile(&ext, Some(&friend), &owner).await.unwrap_err().reason, "USER_NOT_FOUND");

    // GetMe carries all of it for the settings screens.
    let me: GetMeResponse =
        ext.app.call("tokuly.identity.v1.AccountService/GetMe", Some(&owner), &GetMeRequest::default()).await.unwrap();
    assert_eq!(me.me.as_option().unwrap().extras.as_option().unwrap().qa_cards.len(), 4);

    // What changed is announced.
    let events = ext.app.events_by("identity.profile.updated", owner.id).await;
    let changed = events.last().unwrap().data::<tokuly_identity::extras::ProfileUpdated>().unwrap().changed;
    for group in ["mbti", "tags", "qa_cards", "visibility", "theme"] {
        assert!(changed.iter().any(|c| c == group), "{group} in {changed:?}");
    }
    assert!(
        !ext.app.events_by("identity.user.updated", owner.id).await.is_empty(),
        "search rebuilds the account's document"
    );
}

#[tokio::test]
async fn moderators_see_everything_and_others_may_not_ask() {
    let ext = start(IdentityConfig::default()).await;
    let owner = ext.app.user("Rin").await;
    let moderator = ext.app.user_with_roles("Mod", &["moderator"]).await;
    let nobody = ext.app.user("Nobody").await;
    let mut visibility = everything_public();
    visibility.long_bio = vis(Visibility::VISIBILITY_PRIVATE);
    update(
        &ext,
        &owner,
        &UpdateProfileExtrasRequest {
            long_bio: Some("ここだけの話".into()),
            visibility: visibility.into(),
            qa_cards: ProfileQaCardList {
                items: vec![card("hooked_on", "", "裏アカ運営", Visibility::VISIBILITY_PRIVATE, true)],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Even across a block: what was written is what is judged.
    ext.social.block(owner.id, moderator.id);
    let request = GetProfileRequest {
        by: Some(get_profile_request::By::UserId(owner.id.to_string())),
        as_moderator: true,
        ..Default::default()
    };
    let response: GetProfileResponse = ext.app.call(GET_PROFILE, Some(&moderator), &request).await.unwrap();
    let extras = response.profile.as_option().unwrap().extras.as_option().unwrap().clone();
    assert_eq!(extras.long_bio, "ここだけの話");
    assert!(extras.qa_cards[0].hidden);
    assert_eq!(extras.visibility.as_option().unwrap().long_bio, Visibility::VISIBILITY_PRIVATE);
    let log: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM security_events WHERE user_id = $1 AND type = 'MODERATOR_PROFILE_VIEW'",
    )
    .bind(moderator.id)
    .fetch_one(&ext.app.core.db)
    .await
    .unwrap();
    assert_eq!(log, 1);
    let err = ext.app.call::<_, GetProfileResponse>(GET_PROFILE, Some(&nobody), &request).await.unwrap_err();
    assert_eq!(err.reason, "DENIED");
    assert_eq!(
        ext.app.call::<_, GetProfileResponse>(GET_PROFILE, None, &request).await.unwrap_err().code,
        "unauthenticated"
    );
}

#[tokio::test]
async fn updates_are_validated_with_reasons() {
    let ext = start(IdentityConfig::default()).await;
    let user = ext.app.user("Val").await;
    let many_tags = ProfileTagList {
        items: (0..31).map(|i| tag(ProfileTagCategory::PROFILE_TAG_CATEGORY_OTHER, &format!("t{i}"))).collect(),
        ..Default::default()
    };
    let cases: Vec<(&str, UpdateProfileExtrasRequest)> = vec![
        ("MBTI_INVALID", UpdateProfileExtrasRequest { mbti: Some("ABCD".into()), ..Default::default() }),
        ("PRONOUNS_INVALID", UpdateProfileExtrasRequest { pronouns: Some("x".repeat(25)), ..Default::default() }),
        ("LONG_BIO_TOO_LONG", UpdateProfileExtrasRequest { long_bio: Some("あ".repeat(2001)), ..Default::default() }),
        (
            "THEME_INVALID",
            UpdateProfileExtrasRequest {
                theme: ProfileTheme { color: "pink".into(), ..Default::default() }.into(),
                ..Default::default()
            },
        ),
        (
            "VISIBILITY_INVALID",
            UpdateProfileExtrasRequest {
                visibility: ProfileFieldVisibility {
                    mbti: vis(Visibility::VISIBILITY_UNLISTED),
                    ..everything_public()
                }
                .into(),
                ..Default::default()
            },
        ),
        ("TOO_MANY_TAGS", UpdateProfileExtrasRequest { tags: many_tags.into(), ..Default::default() }),
        (
            "TAG_INVALID",
            UpdateProfileExtrasRequest {
                tags: ProfileTagList {
                    items: vec![tag(ProfileTagCategory::PROFILE_TAG_CATEGORY_UNSPECIFIED, "x")],
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
        (
            "LINK_INVALID",
            UpdateProfileExtrasRequest {
                links: ProfileLinkList {
                    items: vec![ProfileLinkInput { url: "javascript:alert(1)".into(), ..Default::default() }],
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
        (
            "TOO_MANY_LINKS",
            UpdateProfileExtrasRequest {
                links: ProfileLinkList {
                    items: (0..9)
                        .map(|i| ProfileLinkInput { url: format!("https://example.com/{i}"), ..Default::default() })
                        .collect(),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
        (
            "OSHI_INVALID",
            UpdateProfileExtrasRequest {
                oshi: OshiList {
                    items: vec![OshiInput { name: " ".into(), ..Default::default() }],
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
        (
            "TOO_MANY_OSHI",
            UpdateProfileExtrasRequest {
                oshi: OshiList {
                    items: (0..11).map(|i| OshiInput { name: format!("推し{i}"), ..Default::default() }).collect(),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
        (
            "PROMPT_NOT_FOUND",
            UpdateProfileExtrasRequest {
                qa_cards: ProfileQaCardList {
                    items: vec![card("nope", "", "a", Visibility::VISIBILITY_PUBLIC, false)],
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
        (
            "CARD_INVALID",
            UpdateProfileExtrasRequest {
                qa_cards: ProfileQaCardList {
                    items: vec![card("", "", "a", Visibility::VISIBILITY_PUBLIC, false)],
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
        (
            "CARD_NOT_FOUND",
            UpdateProfileExtrasRequest {
                qa_cards: ProfileQaCardList {
                    items: vec![ProfileQaCardInput {
                        id: uuid::Uuid::now_v7().to_string(),
                        ..card("dream", "", "a", Visibility::VISIBILITY_PUBLIC, false)
                    }],
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
    ];
    for (reason, request) in cases {
        let err = update(&ext, &user, &request).await.unwrap_err();
        assert_eq!(err.reason, reason, "{err}");
    }
    // Someone else's card cannot be adopted by its ID.
    let other = ext.app.user("Other").await;
    let saved = update(
        &ext,
        &other,
        &UpdateProfileExtrasRequest {
            qa_cards: ProfileQaCardList {
                items: vec![card("dream", "", "宇宙飛行士", Visibility::VISIBILITY_PUBLIC, false)],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let theirs = ProfileQaCardInput {
        id: saved.qa_cards[0].id.clone(),
        ..card("dream", "", "乗っ取り", Visibility::VISIBILITY_PUBLIC, false)
    };
    let err = update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest {
            qa_cards: ProfileQaCardList { items: vec![theirs], ..Default::default() }.into(),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.reason, "CARD_NOT_FOUND");
    // Anonymous callers edit nothing.
    let err = ext
        .app
        .call::<_, UpdateProfileExtrasResponse>(UPDATE, None, &UpdateProfileExtrasRequest::default())
        .await
        .unwrap_err();
    assert_eq!(err.code, "unauthenticated");
}

#[tokio::test]
async fn cards_reorder_keep_their_ids_and_groups_left_out_stay() {
    let ext = start(IdentityConfig::default()).await;
    let user = ext.app.user("Order").await;
    let saved = update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest {
            mbti: Some("INTJ".into()),
            qa_cards: ProfileQaCardList {
                items: vec![
                    card("fav_food", "", "カレー", Visibility::VISIBILITY_PUBLIC, false),
                    card("club", "", "軽音部", Visibility::VISIBILITY_PUBLIC, false),
                ],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (food, club) = (saved.qa_cards[0].id.clone(), saved.qa_cards[1].id.clone());
    let reordered = update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest {
            qa_cards: ProfileQaCardList {
                items: vec![
                    ProfileQaCardInput {
                        id: club.clone(),
                        ..card("club", "", "軽音部", Visibility::VISIBILITY_FRIENDS, false)
                    },
                    ProfileQaCardInput {
                        id: food.clone(),
                        ..card("fav_food", "", "カレー", Visibility::VISIBILITY_PUBLIC, true)
                    },
                ],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(reordered.qa_cards.iter().map(|c| c.id.clone()).collect::<Vec<_>>(), [club, food]);
    assert!(reordered.qa_cards[1].hidden);
    assert_eq!(reordered.mbti, "INTJ", "a group left out of the request stays");
    let emptied = update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest {
            mbti: Some(String::new()),
            qa_cards: ProfileQaCardList::default().into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(emptied.qa_cards.is_empty() && emptied.mbti.is_empty());
}

async fn held_by(ext: &Ext, asset_id: uuid::Uuid, holder: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM media_asset_holders WHERE asset_id = $1 AND holder = $2)")
        .bind(asset_id)
        .bind(holder)
        .fetch_one(&ext.app.core.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn oshi_images_are_held_and_let_go() {
    let ext = start(IdentityConfig::default()).await;
    let user = ext.app.user("Oshi").await;
    let image =
        tokuly_media::testing::ready_image(&ext.app.core, user.id, AssetPurpose::ASSET_PURPOSE_AVATAR).await.unwrap();
    let saved = update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest {
            oshi: OshiList {
                items: vec![
                    OshiInput {
                        name: "推しA".into(),
                        image_asset_id: Some(image.id.to_string()),
                        url: "https://example.com/a".into(),
                        ..Default::default()
                    },
                    OshiInput { name: "推しB".into(), emoji: "🐱".into(), ..Default::default() },
                ],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(saved.oshi[0].image.as_option().is_some());
    let holder = format!("identity:oshi:{}", saved.oshi[0].id);
    assert!(held_by(&ext, image.id, &holder).await);

    // Someone else's upload cannot be used.
    let stranger = ext.app.user("Thief").await;
    let err = update(
        &ext,
        &stranger,
        &UpdateProfileExtrasRequest {
            oshi: OshiList {
                items: vec![OshiInput {
                    name: "x".into(),
                    image_asset_id: Some(image.id.to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "not_found");

    // Kept without naming the image: it stays. Dropped: it is let go.
    let kept = update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest {
            oshi: OshiList {
                items: vec![OshiInput { id: saved.oshi[0].id.clone(), name: "推しA!".into(), ..Default::default() }],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(kept.oshi.len(), 1);
    assert!(kept.oshi[0].image.as_option().is_some());
    update(&ext, &user, &UpdateProfileExtrasRequest { oshi: OshiList::default().into(), ..Default::default() })
        .await
        .unwrap();
    assert!(!held_by(&ext, image.id, &holder).await);
}

#[tokio::test]
async fn concurrent_saves_of_one_account_neither_deadlock_nor_interleave() {
    let ext = start(IdentityConfig::default()).await;
    let user = ext.app.user("Busy").await;
    // As many saves at once as the test pool has connections.
    let saves = (0..5).map(|i| {
        let request = UpdateProfileExtrasRequest {
            pronouns: Some(format!("p{i}")),
            tags: ProfileTagList {
                items: (0..5)
                    .map(|j| tag(ProfileTagCategory::PROFILE_TAG_CATEGORY_GAMES, &format!("game{i}-{j}")))
                    .collect(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        let ext = &ext;
        let user = &user;
        async move { update(ext, user, &request).await }
    });
    let results = futures::future::join_all(saves).await;
    assert!(
        results.iter().all(|r| r.is_ok()),
        "{:?}",
        results.iter().filter_map(|r| r.as_ref().err()).collect::<Vec<_>>()
    );
    let me: GetMeResponse =
        ext.app.call("tokuly.identity.v1.AccountService/GetMe", Some(&user), &GetMeRequest::default()).await.unwrap();
    let extras = me.me.as_option().unwrap().extras.as_option().unwrap().clone();
    // One save won as a whole: its tags and its pronouns.
    let winner = extras.pronouns.trim_start_matches('p').to_owned();
    assert_eq!(extras.tags.len(), 5);
    assert!(extras.tags.iter().all(|t| t.name.starts_with(&format!("game{winner}-"))), "{extras:?}");
}

#[tokio::test]
async fn tag_suggestions_count_public_tags_of_people() {
    let ext = start(IdentityConfig::default()).await;
    let word = unique("ｻﾝﾌﾟﾙ");
    let folded_prefix: String = tokuly_identity::extras::tag_key(&word).chars().take(4).collect();
    let music = ProfileTagCategory::PROFILE_TAG_CATEGORY_MUSIC;
    for (name, public) in [("A", true), ("B", true), ("C", false)] {
        let user = ext.app.user(name).await;
        let mut visibility = everything_public();
        if !public {
            visibility.tags = vis(Visibility::VISIBILITY_FRIENDS);
        }
        update(
            &ext,
            &user,
            &UpdateProfileExtrasRequest {
                visibility: visibility.into(),
                tags: ProfileTagList { items: vec![tag(music, &word)], ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    // A bot showing it does not count.
    let owner = ext.app.user("Owner").await;
    let mut tx = ext.app.core.db.begin().await.unwrap();
    let bot = tokuly_identity::bots::create_bot_account(&mut tx, owner.id, &unique("bot"), "Bot", "").await.unwrap();
    tx.commit().await.unwrap();
    sqlx::query("INSERT INTO profile_extras (user_id) VALUES ($1)").bind(bot).execute(&ext.app.core.db).await.unwrap();
    sqlx::query("INSERT INTO profile_tags (user_id, category, name, name_key, position) VALUES ($1, $2, $3, $4, 0)")
        .bind(bot)
        .bind(music as i32)
        .bind(&word)
        .bind(tokuly_identity::extras::tag_key(&word))
        .execute(&ext.app.core.db)
        .await
        .unwrap();

    let response: SuggestProfileTagsResponse = ext
        .app
        .call(
            "tokuly.identity.v1.ProfileService/SuggestProfileTags",
            None,
            &SuggestProfileTagsRequest {
                category: music.into(),
                query: folded_prefix.to_uppercase(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let hit = response
        .suggestions
        .iter()
        .find(|s| tokuly_identity::extras::tag_key(&s.name) == tokuly_identity::extras::tag_key(&word))
        .unwrap();
    assert_eq!(hit.count, 2, "two people show it publicly");
    let err = ext
        .app
        .call::<_, SuggestProfileTagsResponse>(
            "tokuly.identity.v1.ProfileService/SuggestProfileTags",
            None,
            &SuggestProfileTagsRequest { category: music.into(), query: "x".repeat(25), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(err.reason, "QUERY_TOO_LONG");
    let prompts: ListProfilePromptsResponse = ext
        .app
        .call("tokuly.identity.v1.ProfileService/ListProfilePrompts", None, &ListProfilePromptsRequest::default())
        .await
        .unwrap();
    assert!(prompts.prompts.iter().any(|p| p.question.as_option().unwrap().ja == "無人島に1つ持っていくなら"));
}

#[tokio::test]
async fn search_terms_carry_only_public_fields() {
    let ext = start(IdentityConfig::default()).await;
    let user = ext.app.user("Find").await;
    let mut visibility = everything_public();
    visibility.oshi = vis(Visibility::VISIBILITY_FRIENDS);
    update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest {
            long_bio: Some("写真部です".into()),
            visibility: visibility.into(),
            tags: ProfileTagList {
                items: vec![tag(ProfileTagCategory::PROFILE_TAG_CATEGORY_GAMES, "スプラ")],
                ..Default::default()
            }
            .into(),
            oshi: OshiList {
                items: vec![OshiInput { name: "ひみつの推し".into(), ..Default::default() }],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let terms = tokuly_identity::extras::search_terms(&ext.app.core, &[user.id]).await.unwrap();
    let terms = &terms[&user.id];
    assert_eq!(terms.text, "写真部です");
    assert_eq!(terms.tags, ["スプラ"]);
}

#[tokio::test]
async fn an_erased_account_takes_its_profile_extras_along() {
    let ext = start(IdentityConfig::default()).await;
    let user = ext.app.user("Gone").await;
    let image =
        tokuly_media::testing::ready_image(&ext.app.core, user.id, AssetPurpose::ASSET_PURPOSE_AVATAR).await.unwrap();
    update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest {
            long_bio: Some("bye".into()),
            oshi: OshiList {
                items: vec![OshiInput {
                    name: "推し".into(),
                    image_asset_id: Some(image.id.to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }
            .into(),
            links: ProfileLinkList {
                items: vec![ProfileLinkInput { url: "https://example.com".into(), ..Default::default() }],
                ..Default::default()
            }
            .into(),
            qa_cards: ProfileQaCardList {
                items: vec![card("dream", "", "x", Visibility::VISIBILITY_PUBLIC, false)],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE users SET deletion_scheduled_at = now() - interval '1 minute' WHERE id = $1")
        .bind(user.id)
        .execute(&ext.app.core.db)
        .await
        .unwrap();
    ext.identity.purge_deleted_accounts().await.unwrap();
    for table in ["profile_extras", "profile_oshi", "profile_tags", "profile_links", "profile_qa_cards"] {
        let left: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE user_id = $1")))
                .bind(user.id)
                .fetch_one(&ext.app.core.db)
                .await
                .unwrap();
        assert_eq!(left, 0, "{table}");
    }
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM media_asset_holders WHERE asset_id = $1")
        .bind(image.id)
        .fetch_one(&ext.app.core.db)
        .await
        .unwrap();
    assert_eq!(held, 0);
}

// ───────────────────────────── links ─────────────────────────────

/// A web page on a local port, for the link check.
async fn serve_page(html: String) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/{*path}",
        axum::routing::get(move || {
            let html = html.clone();
            async move { ([("content-type", "text/html; charset=utf-8")], html) }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

#[tokio::test]
async fn links_are_verified_by_a_link_back_and_keep_it_while_their_url_stays() {
    let ext = start(IdentityConfig { profile_links_allow_loopback: true, ..Default::default() }).await;
    let user = ext.app.user("Linker").await;
    let profile_url = format!("http://localhost:8080/u/{}", user.handle);
    let good = serve_page(format!(r#"<html><head><link rel="me" href="{profile_url}"></head></html>"#)).await;
    let bad = serve_page(r#"<a rel="me" href="http://localhost:8080/u/someone_else">x</a>"#.to_owned()).await;
    let links = |urls: &[&str]| ProfileLinkList {
        items: urls
            .iter()
            .map(|u| ProfileLinkInput { title: String::new(), url: (*u).to_owned(), ..Default::default() })
            .collect(),
        ..Default::default()
    };
    let good_page = format!("{good}/me");
    let bad_page = format!("{bad}/me");
    let saved = update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest { links: links(&[&good_page, &bad_page]).into(), ..Default::default() },
    )
    .await
    .unwrap();
    assert!(saved.links.iter().all(|l| !l.verified), "checked in the background");
    assert_eq!(ext.identity.check_profile_links(user.id).await.unwrap(), 2);
    let me = ext.identity.own_extras(user.id, None).await.unwrap();
    assert!(me.links[0].verified && !me.links[1].verified, "{:?}", me.links);
    assert!(me.links[0].title.starts_with("127.0.0.1"), "the host stands in for a missing title");
    // Nothing is due again right away.
    assert_eq!(ext.identity.check_profile_links(user.id).await.unwrap(), 0);
    // Reordered, the verified link stays verified.
    let saved = update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest { links: links(&[&bad_page, &good_page]).into(), ..Default::default() },
    )
    .await
    .unwrap();
    assert!(saved.links[1].verified);
    // A new handle moves the profile URL: the link back no longer matches.
    let _: UpdateProfileResponse = ext
        .app
        .call(
            "tokuly.identity.v1.AccountService/UpdateProfile",
            Some(&user),
            &UpdateProfileRequest { handle: Some(unique("moved")), ..Default::default() },
        )
        .await
        .unwrap();
    ext.identity.check_profile_links(user.id).await.unwrap();
    assert!(ext.identity.own_extras(user.id, None).await.unwrap().links.iter().all(|l| !l.verified));
    let _: RecheckProfileLinksResponse = ext
        .app
        .call(
            "tokuly.identity.v1.AccountService/RecheckProfileLinks",
            Some(&user),
            &RecheckProfileLinksRequest::default(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn the_link_check_does_not_reach_private_addresses() {
    // Loopback is not allowed by default: the page is never fetched.
    let ext = start(IdentityConfig::default()).await;
    let user = ext.app.user("Ssrf").await;
    let page = serve_page(format!(r#"<link rel="me" href="http://localhost:8080/u/{}">"#, user.handle)).await;
    update(
        &ext,
        &user,
        &UpdateProfileExtrasRequest {
            links: ProfileLinkList {
                items: vec![ProfileLinkInput { url: format!("{page}/"), ..Default::default() }],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    ext.identity.check_profile_links(user.id).await.unwrap();
    assert!(!ext.identity.own_extras(user.id, None).await.unwrap().links[0].verified);
}

// ───────────────────────────── pinned song ─────────────────────────────

#[tokio::test]
async fn a_song_is_pinned_from_spotify() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let api = axum::Router::new()
        .route(
            "/v1/tracks/{id}",
            axum::routing::get(|axum::extract::Path(id): axum::extract::Path<String>| async move {
                if id == "0000000000000000000000" {
                    return (axum::http::StatusCode::NOT_FOUND, axum::Json(serde_json::json!({})));
                }
                (
                    axum::http::StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "id": id, "name": "晴る", "artists": [{ "name": "ヨルシカ" }],
                        "album": { "name": "晴る", "images": [{ "url": "https://i.scdn.co/image/a" }] },
                        "external_urls": { "spotify": format!("https://open.spotify.com/track/{id}") }
                    })),
                )
            }),
        )
        .route("/v1/me/player/currently-playing", axum::routing::get(|| async { axum::http::StatusCode::NO_CONTENT }));
    tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });
    let cfg = IdentityConfig {
        spotify_client_id: "id".into(),
        spotify_client_secret: "secret".into(),
        spotify_api_url: format!("http://{address}"),
        ..Default::default()
    };
    let ext = start(cfg).await;
    let user = ext.app.user("Music").await;
    let pin = |track: &str| PinSongRequest { track: track.into(), ..Default::default() };
    const PIN: &str = "tokuly.identity.v1.AccountService/PinSong";
    let err = ext.app.call::<_, PinSongResponse>(PIN, Some(&user), &pin("")).await.unwrap_err();
    assert_eq!(err.reason, "SPOTIFY_NOT_CONNECTED");
    sqlx::query(
        "INSERT INTO connections (user_id, provider, account_name, access_token) VALUES ($1, 'spotify', 'me', $2)",
    )
    .bind(user.id)
    .bind(ext.app.core.keys.seal("connection", b"spotify-token"))
    .execute(&ext.app.core.db)
    .await
    .unwrap();
    let err =
        ext.app.call::<_, PinSongResponse>(PIN, Some(&user), &pin("https://evil.example/track/x")).await.unwrap_err();
    assert_eq!(err.reason, "TRACK_INVALID");
    let err = ext.app.call::<_, PinSongResponse>(PIN, Some(&user), &pin("0000000000000000000000")).await.unwrap_err();
    assert_eq!(err.reason, "TRACK_INVALID");
    let err = ext.app.call::<_, PinSongResponse>(PIN, Some(&user), &pin("")).await.unwrap_err();
    assert_eq!(err.reason, "NOTHING_PLAYING");
    let pinned: PinSongResponse = ext
        .app
        .call(PIN, Some(&user), &pin("https://open.spotify.com/intl-ja/track/4uLU6hMCjMI75M1A2tKUQC?si=1"))
        .await
        .unwrap();
    assert_eq!(pinned.song.as_option().unwrap().artist, "ヨルシカ");
    let stranger = ext.app.user("Listener").await;
    let seen = profile(&ext, Some(&stranger), &user).await.unwrap();
    assert_eq!(seen.extras.as_option().unwrap().pinned_song.as_option().unwrap().title, "晴る");
    let _: PinSongResponse =
        ext.app.call(PIN, Some(&user), &PinSongRequest { clear: true, ..Default::default() }).await.unwrap();
    let seen = profile(&ext, Some(&stranger), &user).await.unwrap();
    assert!(seen.extras.as_option().unwrap().pinned_song.as_option().is_none());
}

// ───────────────────────────── bots ─────────────────────────────

#[tokio::test]
async fn bots_are_marked_on_profiles_and_never_count_as_people() {
    let ext = start(IdentityConfig::default()).await;
    let owner = ext.app.user("Maker").await;
    let viewer = ext.app.user("Viewer").await;
    let handle = unique("helperbot");
    let mut tx = ext.app.core.db.begin().await.unwrap();
    let bot_id =
        tokuly_identity::bots::create_bot_account(&mut tx, owner.id, &handle, "おてつだいBot", "").await.unwrap();
    tx.commit().await.unwrap();
    let bot = ext.app.sign_in(bot_id).await;

    let seen = profile(&ext, Some(&viewer), &bot).await.unwrap();
    assert!(seen.bot);
    assert_eq!(seen.bot_owner.as_option().unwrap().id, owner.id.to_string());
    // The person behind it stays out of sight across a block.
    ext.social.block(owner.id, viewer.id);
    let seen = profile(&ext, Some(&viewer), &bot).await.unwrap();
    assert!(seen.bot && seen.bot_owner.as_option().is_none());

    let core = &ext.app.core;
    assert!(tokuly_identity::bots::is_bot(core, bot_id).await.unwrap());
    assert_eq!(
        tokuly_identity::bots::humans_only(core, &[viewer.id, bot_id, owner.id]).await.unwrap(),
        [viewer.id, owner.id]
    );
    assert_eq!(tokuly_identity::bots::bot_owners(core, &[bot_id, owner.id]).await.unwrap()[&bot_id], owner.id);

    // Handed over; a bot cannot own a bot; the handle is taken.
    let mut tx = core.db.begin().await.unwrap();
    tokuly_identity::bots::set_bot_owner(&mut tx, bot_id, viewer.id).await.unwrap();
    assert_eq!(
        tokuly_identity::bots::set_bot_owner(&mut tx, bot_id, bot_id).await.unwrap_err().reason,
        "OWNER_INVALID"
    );
    assert_eq!(
        tokuly_identity::bots::create_bot_account(&mut tx, owner.id, &handle, "x", "").await.unwrap_err().reason,
        "HANDLE_TAKEN"
    );
    tx.rollback().await.unwrap();

    let me: GetMeResponse =
        ext.app.call("tokuly.identity.v1.AccountService/GetMe", Some(&bot), &GetMeRequest::default()).await.unwrap();
    let me = me.me.into_option().unwrap();
    assert!(me.bot);
    assert_eq!(me.bot_owner.as_option().unwrap().id, owner.id.to_string());
    let found: SearchUsersResponse = ext
        .app
        .call(
            "tokuly.identity.v1.ProfileService/SearchUsers",
            Some(&owner),
            &SearchUsersRequest { query: handle.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(found.results.iter().any(|r| r.bot));
}

// ───────────────────────────── birthdays ─────────────────────────────

async fn set_privacy(
    ext: &Ext,
    user: &TestUser,
    change: impl FnOnce(&mut PrivacySettings),
) -> Result<Settings, tokuly_testkit::RpcError> {
    let current: GetSettingsResponse = ext
        .app
        .call("tokuly.identity.v1.AccountService/GetSettings", Some(user), &GetSettingsRequest::default())
        .await
        .unwrap();
    let mut privacy = current.settings.as_option().unwrap().privacy.as_option().unwrap().clone();
    change(&mut privacy);
    let response: UpdateSettingsResponse = ext
        .app
        .call(
            "tokuly.identity.v1.AccountService/UpdateSettings",
            Some(user),
            &UpdateSettingsRequest { privacy: privacy.into(), ..Default::default() },
        )
        .await?;
    Ok(response.settings.into_option().unwrap())
}

#[tokio::test]
async fn birthdays_follow_the_one_setting_everywhere() {
    let ext = start(IdentityConfig::default()).await;
    let owner = ext.app.user("Bday").await; // born 2000-01-01
    let friend = ext.app.user("Pal").await;
    let stranger = ext.app.user("Far").await;
    ext.social.befriend(owner.id, friend.id);

    // Nobody by default.
    assert!(profile(&ext, Some(&friend), &owner).await.unwrap().birthday.as_option().is_none());
    assert_eq!(profile(&ext, Some(&owner), &owner).await.unwrap().birthday.as_option().unwrap().year, 2000);

    set_privacy(&ext, &owner, |p| p.birthday_visible_to = Audience::AUDIENCE_FRIENDS.into()).await.unwrap();
    let seen = profile(&ext, Some(&friend), &owner).await.unwrap();
    let birthday = seen.birthday.as_option().unwrap();
    assert_eq!((birthday.month, birthday.day, birthday.year), (1, 1, 0));
    assert!(profile(&ext, Some(&stranger), &owner).await.unwrap().birthday.as_option().is_none());

    set_privacy(&ext, &owner, |p| {
        p.birthday_visible_to = Audience::AUDIENCE_EVERYONE.into();
        p.birthday_show_year = true;
    })
    .await
    .unwrap();
    assert_eq!(profile(&ext, None, &owner).await.unwrap().birthday.as_option().unwrap().year, 2000);
    // A client that does not know the setting keeps it.
    let settings =
        set_privacy(&ext, &owner, |p| p.birthday_visible_to = Audience::AUDIENCE_UNSPECIFIED.into()).await.unwrap();
    assert_eq!(settings.privacy.as_option().unwrap().birthday_visible_to, Audience::AUDIENCE_EVERYONE);
    let err = set_privacy(&ext, &owner, |p| p.birthday_visible_to = Audience::AUDIENCE_FRIENDS_OF_FRIENDS.into())
        .await
        .unwrap_err();
    assert_eq!(err.reason, "SETTINGS_INVALID");

    // Batched, with blocks taken into account.
    let seen = ext.identity.visible_birthdays(Some(stranger.id), &[owner.id, friend.id]).await.unwrap();
    assert!(seen.contains_key(&owner.id) && !seen.contains_key(&friend.id));
    ext.social.block(stranger.id, owner.id);
    assert!(ext.identity.visible_birthdays(Some(stranger.id), &[owner.id]).await.unwrap().is_empty());
    assert!(!ext.app.events_by("identity.user.privacy_changed", owner.id).await.is_empty());

    // Under 18 the year is not shown to everyone.
    let minor = ext.app.user("Kid").await;
    let born = chrono::Utc::now().date_naive() - chrono::Duration::days(15 * 365);
    sqlx::query("UPDATE users SET birthdate = $2 WHERE id = $1")
        .bind(minor.id)
        .bind(born)
        .execute(&ext.app.core.db)
        .await
        .unwrap();
    let err = set_privacy(&ext, &minor, |p| {
        p.birthday_visible_to = Audience::AUDIENCE_EVERYONE.into();
        p.birthday_show_year = true;
    })
    .await
    .unwrap_err();
    assert_eq!(err.reason, "BIRTHDAY_YEAR_RESTRICTED");
    let err = tokuly_identity::birthdays::set_birthday_audience(
        &ext.app.core,
        minor.id,
        Audience::AUDIENCE_EVERYONE,
        Some(true),
    )
    .await
    .unwrap_err();
    assert_eq!(err.reason, "BIRTHDAY_YEAR_RESTRICTED");
    tokuly_identity::birthdays::set_birthday_audience(&ext.app.core, minor.id, Audience::AUDIENCE_FRIENDS, Some(true))
        .await
        .unwrap();
}

#[tokio::test]
async fn the_zodiac_is_derived_and_starts_private() {
    let ext = start(IdentityConfig::default()).await;
    let owner = ext.app.user("Stars").await; // 2000-01-01: capricorn
    let stranger = ext.app.user("Look").await;
    assert_eq!(
        profile(&ext, Some(&stranger), &owner).await.unwrap().extras.as_option().unwrap().zodiac,
        Zodiac::ZODIAC_UNSPECIFIED
    );
    assert_eq!(
        profile(&ext, Some(&owner), &owner).await.unwrap().extras.as_option().unwrap().zodiac,
        Zodiac::ZODIAC_CAPRICORN
    );
    update(&ext, &owner, &UpdateProfileExtrasRequest { visibility: everything_public().into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(
        profile(&ext, Some(&stranger), &owner).await.unwrap().extras.as_option().unwrap().zodiac,
        Zodiac::ZODIAC_CAPRICORN
    );
}
