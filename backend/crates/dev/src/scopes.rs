//! The scope catalogue: every scope a token can hold, in words a person
//! reads before agreeing to it.
//!
//! What a scope *does* is decided in core, RPC by RPC
//! (`tokuly_core::api_access`). This file names the scopes tokens may be
//! given and says, per scope, what that means for the person. A scope is
//! offered only if some classified RPC needs it (a test holds the two
//! together). The wording is part of the security of the consent screen:
//! it has to say what is shared and what stays out of reach, not what the
//! API is called.
use tokuly_core::{api_tokens::Scopes, prelude::*};
use tokuly_proto::{common::LocalizedText, dev::ScopeInfo};

struct Domain {
    name: &'static str,
    /// Location, private conversations, money, the account itself.
    sensitive: bool,
    /// (ja, en) for `:read`.
    read: (&'static str, &'static str),
    /// (ja, en) for `:write`, which includes reading. `None` where no RPC
    /// of the domain can be called with a write scope.
    write: Option<(&'static str, &'static str)>,
}

/// Domains a token can be scoped to, most sensitive first. `admin` is
/// absent on purpose: no token can moderate.
const DOMAINS: &[Domain] = &[
    Domain {
        name: "geo",
        sensitive: true,
        read: (
            "あなたの現在地と、友達があなたに共有している現在地をリアルタイムで見る。位置履歴・よく行く場所・移動ルートも含まれます",
            "See your live location and the live locations your friends share with you, in real time, plus your location history, places and routes",
        ),
        write: Some((
            "あなたの現在地を更新・共有する、ゴーストモードや場所・見守りアラートを変更する、位置履歴を削除する（位置情報の閲覧を含む）。SOSの発信・解除はできません",
            "Update and share your location, change ghost mode, places and place alerts, delete location history (includes seeing all location data). Never sends or ends an SOS",
        )),
    },
    Domain {
        name: "chat",
        sensitive: true,
        read: (
            "DMとグループのメッセージ・メンバー・通話履歴を読む",
            "Read your direct and group messages, their members and your call history",
        ),
        write: Some((
            "あなたとしてメッセージを送信・編集・削除する、グループを作成・管理する、通話を開始する",
            "Send, edit and delete messages as you, create and manage groups, start calls",
        )),
    },
    Domain {
        name: "identity",
        sensitive: true,
        read: (
            "プロフィール・メールアドレス・設定を見る、ユーザーを検索する。友達との距離を含むプロフィール詳細には位置情報の許可も必要です",
            "See your profile, email address and settings; look up and search users. Full profiles, which show how far away a friend is, also take the location permission",
        ),
        write: Some((
            "プロフィール・ステータス・プライバシー設定を変更する（パスワード・メールアドレス・外部サービス連携は変更できません）",
            "Change your profile, status and privacy settings (never your password, email address or linked services)",
        )),
    },
    Domain {
        name: "wallet",
        sensitive: true,
        read: ("コイン残高・取引履歴・収益を見る", "See your coin balance, transaction history and earnings"),
        write: Some((
            "コインを使う・デイリーボーナスを受け取る（コインの購入や出金はできません）",
            "Spend coins and claim the daily bonus (never buys coins or cashes out)",
        )),
    },
    Domain {
        name: "social",
        sensitive: true,
        read: (
            "友達・友達リクエスト・ブロックリスト・招待リンクを見る",
            "See your friends, friend requests, block list and invite link",
        ),
        write: Some((
            "友達リクエストの送信・承認、友達の削除、ブロック、バンプやノックの送信",
            "Send and answer friend requests, remove friends, block people, send bumps and knocks",
        )),
    },
    Domain {
        name: "notify",
        // The inbox repeats what other domains sent: who wrote, where a friend arrived.
        sensitive: true,
        read: (
            "通知設定と未読数を見る。通知の内容（メッセージの冒頭や見守りアラートを含む）を読むには、チャットと位置情報の許可も必要です",
            "See your notification settings and unread count. Reading the notifications themselves, which quote messages and place alerts, also takes the chat and location permissions",
        ),
        write: Some((
            "通知を既読・削除する、通知設定やミュートを変更する",
            "Mark notifications read, delete them, change notification settings and mutes",
        )),
    },
    Domain {
        name: "notes",
        // Notes and stories can be tagged with the place they were posted from.
        sensitive: true,
        read: (
            "あなたが見られるノート・ストーリー・タイムラインを読む。投稿に付けられた場所（位置情報）も含まれます",
            "Read the notes, stories and timelines you can see, including the places posts are tagged with",
        ),
        write: Some((
            "あなたとしてノートやストーリーを投稿・削除する、いいね・リノートする",
            "Post and delete notes and stories as you, like and renote",
        )),
    },
    Domain {
        name: "spaces",
        sensitive: true,
        read: (
            "スペースとたまり場、そして今だれが中にいるかを見る",
            "See audio rooms and hangouts, and who is inside them right now",
        ),
        write: Some((
            "スペースの予定の変更・終了・進行、たまり場の作成と管理、招待（入室して聞く・話すことはできません）",
            "Change, end and moderate your rooms, create and manage hangouts, invite people (cannot enter a room to listen or talk)",
        )),
    },
    Domain {
        name: "live",
        // Who watches which stream, and who is in whose chat.
        sensitive: true,
        read: (
            "ライブ配信・チャット・アンケート・クリップと、あなたの配信の視聴者リストや分析を見る（配信キーの取得や限定公開の配信の再生はできません。ギフトの履歴にはコインの許可も必要です）",
            "See live streams, their chat, polls and clips, and who is watching your own streams and how they did (never your stream key, and it cannot play a restricted stream; gift history also takes the coin permission)",
        ),
        write: Some((
            "配信の作成・編集・終了・削除、あなたとしてチャットに投稿、チャットのモデレーション、アンケート、クリップの作成（配信の開始、配信キーやオーバーレイの管理、モデレーターの任命はできません。ギフトを送るにはコインの許可も必要です）",
            "Create, edit, end and delete your streams, chat as you, moderate chats, run polls and cut clips (cannot go on air, get your stream key, manage overlays or appoint moderators; sending gifts also takes the coin permission)",
        )),
    },
    Domain {
        name: "feed",
        sensitive: false,
        read: (
            "ニュースとフィードの設定を見る。アクティビティログ・友達のフィード・マップストーリー（友達がいた場所を含む）には位置情報の許可も必要です",
            "See news and feed settings. The activity log, your friends' feed and map stories, which show where friends were, also take the location permission",
        ),
        write: Some((
            "アクティビティの削除や公開範囲の変更、ニュースの好みを変更する",
            "Delete activity entries, change who sees them, tune news preferences",
        )),
    },
    Domain {
        name: "video",
        sensitive: false,
        read: (
            "チャンネル・動画・再生リスト・視聴履歴・アナリティクスを見る",
            "See channels, videos, playlists, your watch history and analytics",
        ),
        write: Some((
            "動画やチャンネルの作成・編集・削除、チャンネル登録、ウォッチパーティーの操作",
            "Create, edit and delete videos and channels, subscribe, run watch parties",
        )),
    },
    Domain {
        name: "engagement",
        sensitive: false,
        read: (
            "コメント・リアクション・ブックマーク・リアクションの設定を見る",
            "See comments, reactions, your bookmarks and your reaction settings",
        ),
        write: Some((
            "あなたとしてコメント・リアクション・ブックマーク・通報をし、あなたの投稿へのリアクションの設定を変える",
            "Comment, react, bookmark and report as you, and change which reactions your posts take",
        )),
    },
    Domain {
        name: "media",
        sensitive: false,
        read: ("アップロードしたファイルの情報を見る", "See the files you uploaded"),
        write: Some(("ファイルをアップロード・削除する", "Upload and delete files")),
    },
    Domain {
        name: "stamp",
        sensitive: false,
        read: ("スタンプショップとあなたのスタンプを見る", "See the stamp store and your stamps"),
        write: Some((
            "スタンプの入手・購入・並べ替え、スタンプパックの作成",
            "Get, buy and arrange stamps, create stamp packs",
        )),
    },
    Domain {
        name: "emoji",
        sensitive: false,
        read: ("カスタム絵文字を見る", "See custom emoji"),
        write: Some(("カスタム絵文字を申請・管理する", "Submit and manage custom emoji")),
    },
    Domain {
        name: "search",
        sensitive: false,
        read: ("検索する、検索履歴を見る", "Search, and see your search history"),
        write: Some(("検索履歴を削除・一時停止する", "Delete or pause your search history")),
    },
    Domain {
        name: "ask",
        sensitive: false,
        read: ("質問箱に届いた質問と回答を見る", "See the questions in your question box and your answers"),
        write: Some((
            "質問を送る・回答する、ほめ投票に参加する",
            "Ask and answer questions, take part in compliment polls",
        )),
    },
    Domain {
        name: "school",
        sensitive: false,
        read: ("時間割・勉強記録・テスト予定を見る", "See your timetable, study logs and exams"),
        write: Some(("時間割・勉強記録・テスト予定を編集する", "Edit your timetable, study logs and exams")),
    },
    Domain {
        name: "plans",
        sensitive: false,
        read: (
            "ひま状況・あそぶ予定・日程調整・記念日・割り勘の記録を見る",
            "See free-time status, plans, date polls, anniversaries and cost splits",
        ),
        write: Some((
            "予定や日程調整の作成・参加・回答、ひま状況の変更、割り勘の記録（コインは動きません）",
            "Create, join and answer plans and date polls, change your free-time status, record cost splits (no coins move)",
        )),
    },
    Domain {
        name: "community",
        sensitive: false,
        read: (
            "参加しているコミュニティとその投稿・イベント・メンバーを見る",
            "See your communities, their posts, events and members",
        ),
        write: Some((
            "コミュニティへの参加・投稿・イベント参加・管理（匿名投稿の投稿者を明かすことはできません）",
            "Join communities, post, attend events and moderate (never reveals who wrote an anonymous post)",
        )),
    },
    Domain {
        name: "shorts",
        sensitive: false,
        read: (
            "ショートのフィード、みんなのショート、保存・いいねしたショート、自分のショートとその分析を見る",
            "See short video feeds, people's shorts, the shorts you saved or liked, your own shorts and their analytics",
        ),
        write: Some((
            "ショートの投稿・編集・削除・保存、視聴の記録、「興味なし」の設定",
            "Post, edit, delete and save shorts, record what you watched, mark shorts as not interesting",
        )),
    },
    Domain {
        name: "quest",
        sensitive: false,
        read: (
            "レベル・XP・ミッション・バッジ・ストリーク・ランキングを見る",
            "See your level, XP, missions, badges, streaks and leaderboards",
        ),
        write: Some((
            "ミッションの入れ替え、バッジと称号の表示設定（報酬の受け取りとストリーク復活にはコインの許可も必要です）",
            "Reroll missions, choose the badges and title you show (claiming rewards and restoring a streak also need the coin permission)",
        )),
    },
    Domain {
        name: "realtime",
        sensitive: false,
        read: ("リアルタイム接続先の情報を取得する", "Look up the realtime endpoints"),
        write: None,
    },
    Domain {
        name: "dev",
        sensitive: false,
        read: (
            "Webhookの一覧とAPI利用状況を見る。配信ログの内容は、そのイベントを読める許可がある場合のみ見られます",
            "See your webhooks and API usage. A delivery in the log can be read only with the permissions its event needs",
        ),
        write: None,
    },
];

fn info(domain: &Domain, write: bool) -> Option<ScopeInfo> {
    let (ja, en) = if write { domain.write? } else { domain.read };
    Some(ScopeInfo {
        scope: format!("{}:{}", domain.name, if write { "write" } else { "read" }),
        domain: domain.name.to_owned(),
        write,
        description: LocalizedText { ja: ja.to_owned(), en: en.to_owned(), ..Default::default() }.into(),
        sensitive: domain.sensitive,
        ..Default::default()
    })
}

/// Every scope, in catalogue order (sensitive domains first, read before write).
pub fn catalogue() -> Vec<ScopeInfo> {
    DOMAINS.iter().flat_map(|domain| [info(domain, false), info(domain, true)]).flatten().collect()
}

/// The catalogue entry of one scope.
pub fn find(scope: &str) -> Option<ScopeInfo> {
    let (domain, level) = scope.split_once(':')?;
    let domain = DOMAINS.iter().find(|d| d.name == domain)?;
    match level {
        "read" => info(domain, false),
        "write" => info(domain, true),
        _ => None,
    }
}

/// Domains a bot may write to: its own profile, uploads, inbox and
/// preferences. Everything that addresses another person — friend requests,
/// messages, notes, comments, reactions — is held back until blocks reach
/// bots: someone who blocked a bot's owner must not be reachable through
/// the bot, and that check belongs to the social domain, which does not
/// know about bots yet.
const BOT_WRITE_DOMAINS: &[&str] = &["identity", "media", "notify", "search", "feed"];

/// Whether a bot token may hold `scope`. Never a wallet scope: a bot is an
/// account anyone can make more of, so it must neither collect coins nor
/// spend them.
pub fn allowed_for_bots(scope: &str) -> bool {
    match scope.split_once(':') {
        Some(("wallet", _)) => false,
        Some((domain, "write")) => BOT_WRITE_DOMAINS.contains(&domain),
        Some((_, "read")) => true,
        _ => false,
    }
}

/// The entries of `scopes` in catalogue order, for the consent screen.
pub fn describe(scopes: &[String]) -> Vec<ScopeInfo> {
    catalogue().into_iter().filter(|info| scopes.contains(&info.scope)).collect()
}

/// Checks scopes named by a client: each must be in the catalogue, and
/// there must be at least one. A read that a write of the same domain
/// already includes is dropped.
pub fn validate(requested: &[String]) -> AppResult<Scopes> {
    ensure!(!requested.is_empty(), invalid, "SCOPES_REQUIRED", "choose at least one scope");
    for scope in requested {
        ensure!(find(scope).is_some(), invalid, "SCOPE_UNKNOWN", "unknown scope {scope:?}");
    }
    Ok(normalize(requested))
}

fn normalize(scopes: &[String]) -> Scopes {
    let implied =
        |scope: &String| scope.strip_suffix(":read").is_some_and(|domain| scopes.contains(&format!("{domain}:write")));
    Scopes::new(scopes.iter().filter(|scope| !implied(scope)).cloned())
}

/// Parses an OAuth `scope` parameter (space separated). `Err` holds the
/// first entry that is not a known scope.
pub fn parse_oauth(scope: &str) -> Result<Scopes, String> {
    let requested: Vec<String> = scope.split_ascii_whitespace().map(str::to_owned).collect();
    match requested.iter().find(|scope| find(scope).is_none()) {
        Some(unknown) => Err(unknown.clone()),
        None => Ok(normalize(&requested)),
    }
}

#[cfg(test)]
mod tests {
    use tokuly_core::api_tokens::{Access, access_for, is_scope};

    use super::*;

    #[test]
    fn the_catalogue_is_well_formed_and_worded_in_both_languages() {
        let all = catalogue();
        assert_eq!(all.len(), DOMAINS.len() * 2 - DOMAINS.iter().filter(|d| d.write.is_none()).count());
        let mut names: Vec<&str> = all.iter().map(|s| s.scope.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len(), "no scope is listed twice");
        for scope in &all {
            assert!(is_scope(&scope.scope), "{}", scope.scope);
            let text = scope.description.as_option().unwrap();
            assert!(!text.ja.is_empty() && !text.en.is_empty(), "{}", scope.scope);
        }
        assert!(find("admin:read").is_none() && find("admin:write").is_none(), "no token moderates");
    }

    #[test]
    fn the_location_scope_says_it_shares_live_location() {
        let geo = find("geo:read").unwrap();
        let text = geo.description.as_option().unwrap();
        assert!(geo.sensitive);
        assert!(text.ja.contains("現在地") && text.ja.contains("リアルタイム"), "{}", text.ja);
        assert!(text.en.contains("live location"), "{}", text.en);
        // And it is the scope the location RPCs actually need.
        assert_eq!(
            access_for("/tokuly.geo.v1.LocationService/WatchFriendLocations"),
            Some(Access::Scopes(vec!["geo:read".into()]))
        );
        assert_eq!(catalogue()[0].scope, "geo:read", "location leads the consent screen");
    }

    #[test]
    fn requested_scopes_must_be_known() {
        let ok = validate(&["notes:read".into(), "notes:write".into(), "geo:read".into()]).unwrap();
        assert_eq!(ok.as_slice(), ["geo:read", "notes:write"], "a read included in a write is dropped");
        assert_eq!(validate(&[]).unwrap_err().reason, "SCOPES_REQUIRED");
        for bad in ["admin:write", "notes", "notes:delete", "nope:read", ""] {
            assert_eq!(validate(&[bad.to_owned()]).unwrap_err().reason, "SCOPE_UNKNOWN", "{bad}");
        }
    }

    #[test]
    fn oauth_scope_strings_are_space_separated() {
        assert_eq!(parse_oauth("notes:read  geo:read").unwrap().to_string(), "geo:read notes:read");
        assert!(parse_oauth("").unwrap().is_empty());
        assert_eq!(parse_oauth("notes:read admin:read"), Err("admin:read".to_owned()));
        let described = describe(&["notes:write".into(), "geo:read".into()]);
        assert_eq!(described.iter().map(|s| s.scope.as_str()).collect::<Vec<_>>(), ["geo:read", "notes:write"]);
    }

    /// A scope is offered only where the API has been gone through: every
    /// scope of the catalogue is what some classified RPC needs, and every
    /// scope an RPC needs can be granted.
    #[test]
    fn the_catalogue_and_the_access_table_agree() {
        let needed: std::collections::BTreeSet<String> = tokuly_core::api_access::classified()
            .filter_map(|(_, access)| match access {
                Access::Scopes(scopes) => Some(scopes),
                _ => None,
            })
            .flatten()
            .collect();
        let offered: std::collections::BTreeSet<String> = catalogue().into_iter().map(|scope| scope.scope).collect();
        assert_eq!(offered, needed);
        for gone in ["live:admin", "dev:write", "admin:read", "spaces:join"] {
            assert!(find(gone).is_none(), "{gone}");
        }
    }

    #[test]
    fn the_wording_says_what_a_scope_reveals_and_what_it_cannot_do() {
        let en = |scope: &str| find(scope).unwrap().description.as_option().unwrap().en.clone();
        assert!(find("notify:read").unwrap().sensitive);
        assert!(en("notify:read").contains("chat and location"));
        assert!(en("feed:read").contains("where friends were"));
        assert!(en("identity:read").contains("how far away"));
        assert!(en("geo:write").contains("Never sends or ends an SOS"));
        assert!(en("identity:write").contains("linked services"));
        assert!(en("wallet:write").contains("never buys coins or cashes out"));
        assert!(find("notes:read").unwrap().sensitive && en("notes:read").contains("places"));
    }

    #[test]
    fn bots_get_no_money_and_no_way_to_address_people() {
        for scope in [
            "wallet:read",
            "wallet:write",
            "chat:write",
            "social:write",
            "notes:write",
            "engagement:write",
            "ask:write",
            "geo:write",
            "video:write",
        ] {
            assert!(!allowed_for_bots(scope), "{scope}");
        }
        for scope in ["chat:read", "notes:read", "geo:read", "identity:write", "media:write", "notify:write"] {
            assert!(allowed_for_bots(scope), "{scope}");
        }
    }
}
