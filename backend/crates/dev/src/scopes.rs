//! The scope catalogue: every scope a token can hold, in words a person
//! reads before agreeing to it.
//!
//! What a scope *does* is decided in core from the RPC path
//! (`tokuly_core::api_tokens::access_for`): `<domain>:read` covers the
//! reading RPCs of the domain's package, `<domain>:write` all of them. This
//! file only names the domains tokens may be given and says, per scope,
//! what that means for the person. The wording is part of the security of
//! the consent screen: it has to say what is shared, not what the API is
//! called.
use tokuly_core::{api_tokens::Scopes, prelude::*};
use tokuly_proto::{common::LocalizedText, dev::ScopeInfo};

struct Domain {
    name: &'static str,
    /// Location, private conversations, money, the account itself.
    sensitive: bool,
    /// (ja, en) for `:read`.
    read: (&'static str, &'static str),
    /// (ja, en) for `:write`, which includes reading.
    write: (&'static str, &'static str),
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
        write: (
            "あなたの現在地を更新・共有する、ゴーストモードや場所・見守りアラートを変更する、位置履歴を削除する（位置情報の閲覧を含む）",
            "Update and share your location, change ghost mode, places and place alerts, delete location history (includes seeing all location data)",
        ),
    },
    Domain {
        name: "chat",
        sensitive: true,
        read: (
            "DMとグループのメッセージ・メンバー・通話履歴を読む",
            "Read your direct and group messages, their members and your call history",
        ),
        write: (
            "あなたとしてメッセージを送信・編集・削除する、グループを作成・管理する、通話を開始する",
            "Send, edit and delete messages as you, create and manage groups, start calls",
        ),
    },
    Domain {
        name: "identity",
        sensitive: true,
        read: (
            "プロフィール・メールアドレス・設定を見る、ユーザーを検索する",
            "See your profile, email address and settings; look up and search users",
        ),
        write: (
            "プロフィール・ステータス・プライバシー設定を変更する（パスワードやメールアドレスは変更できません）",
            "Change your profile, status and privacy settings (never your password or email address)",
        ),
    },
    Domain {
        name: "wallet",
        sensitive: true,
        read: ("コイン残高・取引履歴・収益を見る", "See your coin balance, transaction history and earnings"),
        write: (
            "コインを使う・購入を開始する・デイリーボーナスを受け取る",
            "Spend coins, start purchases and claim the daily bonus",
        ),
    },
    Domain {
        name: "social",
        sensitive: true,
        read: (
            "友達・友達リクエスト・ブロックリスト・招待リンクを見る",
            "See your friends, friend requests, block list and invite link",
        ),
        write: (
            "友達リクエストの送信・承認、友達の削除、ブロック、バンプやノックの送信",
            "Send and answer friend requests, remove friends, block people, send bumps and knocks",
        ),
    },
    Domain {
        name: "notify",
        sensitive: false,
        read: ("通知と通知設定を見る", "Read your notifications and notification settings"),
        write: (
            "通知を既読・削除する、通知設定やミュートを変更する",
            "Mark notifications read, delete them, change notification settings and mutes",
        ),
    },
    Domain {
        name: "notes",
        sensitive: false,
        read: (
            "あなたが見られるノート・ストーリー・タイムラインを読む",
            "Read the notes, stories and timelines you can see",
        ),
        write: (
            "あなたとしてノートやストーリーを投稿・削除する、いいね・リノートする",
            "Post and delete notes and stories as you, like and renote",
        ),
    },
    Domain {
        name: "feed",
        sensitive: false,
        read: (
            "アクティビティログ・友達のフィード・ニュースを見る",
            "See your activity log, your friends' feed and news",
        ),
        write: (
            "アクティビティの削除や公開範囲の変更、ニュースの好みを変更する",
            "Delete activity entries, change who sees them, tune news preferences",
        ),
    },
    Domain {
        name: "video",
        sensitive: false,
        read: (
            "チャンネル・動画・再生リスト・視聴履歴・アナリティクスを見る",
            "See channels, videos, playlists, your watch history and analytics",
        ),
        write: (
            "動画やチャンネルの作成・編集・削除、チャンネル登録、ウォッチパーティーの操作",
            "Create, edit and delete videos and channels, subscribe, run watch parties",
        ),
    },
    Domain {
        name: "live",
        sensitive: false,
        read: ("ライブ配信とそのチャットを見る", "See live streams and their chat"),
        write: (
            "ライブ配信を開始・管理する、ライブチャットに書き込む、ギフトを送る",
            "Start and manage live streams, write in live chat, send gifts",
        ),
    },
    Domain {
        name: "shorts",
        sensitive: false,
        read: ("ショート動画とフィードを見る", "See shorts and the shorts feed"),
        write: ("ショート動画を投稿・編集・削除する", "Post, edit and delete shorts"),
    },
    Domain {
        name: "engagement",
        sensitive: false,
        read: ("コメント・リアクション・ブックマークを見る", "See comments, reactions and your bookmarks"),
        write: (
            "あなたとしてコメント・リアクション・ブックマーク・通報をする",
            "Comment, react, bookmark and report as you",
        ),
    },
    Domain {
        name: "media",
        sensitive: false,
        read: ("アップロードしたファイルの情報を見る", "See the files you uploaded"),
        write: ("ファイルをアップロード・削除する", "Upload and delete files"),
    },
    Domain {
        name: "stamp",
        sensitive: false,
        read: ("スタンプショップとあなたのスタンプを見る", "See the stamp store and your stamps"),
        write: (
            "スタンプの入手・購入・並べ替え、スタンプパックの作成",
            "Get, buy and arrange stamps, create stamp packs",
        ),
    },
    Domain {
        name: "emoji",
        sensitive: false,
        read: ("カスタム絵文字を見る", "See custom emoji"),
        write: ("カスタム絵文字を申請・管理する", "Submit and manage custom emoji"),
    },
    Domain {
        name: "search",
        sensitive: false,
        read: ("検索する、検索履歴を見る", "Search, and see your search history"),
        write: ("検索履歴を削除・一時停止する", "Delete or pause your search history"),
    },
    Domain {
        name: "ask",
        sensitive: false,
        read: ("質問箱に届いた質問と回答を見る", "See the questions in your question box and your answers"),
        write: ("質問を送る・回答する、ほめ投票に参加する", "Ask and answer questions, take part in compliment polls"),
    },
    Domain {
        name: "quest",
        sensitive: false,
        read: ("ミッション・バッジ・レベル・ストリークを見る", "See your missions, badges, level and streaks"),
        write: ("ミッションの報酬を受け取る", "Claim mission rewards"),
    },
    Domain {
        name: "plans",
        sensitive: false,
        read: ("ひま状況・あそぶ予定・日程調整を見る", "See free-time status, plans and date polls"),
        write: (
            "予定の作成・参加・回答、ひま状況の変更",
            "Create, join and answer plans, change your free-time status",
        ),
    },
    Domain {
        name: "school",
        sensitive: false,
        read: ("時間割・勉強記録・テスト予定を見る", "See your timetable, study logs and exams"),
        write: ("時間割・勉強記録・テスト予定を編集する", "Edit your timetable, study logs and exams"),
    },
    Domain {
        name: "spaces",
        sensitive: false,
        read: ("たまり場（音声ルーム）を見る", "See audio rooms"),
        write: ("たまり場を作成・参加・管理する", "Create, join and manage audio rooms"),
    },
    Domain {
        name: "community",
        sensitive: false,
        read: ("参加しているコミュニティとその投稿を見る", "See your communities and their posts"),
        write: ("コミュニティへの参加・投稿・管理", "Join, post in and manage communities"),
    },
    Domain {
        name: "realtime",
        sensitive: false,
        read: ("リアルタイム接続先の情報を取得する", "Look up the realtime endpoints"),
        write: ("リアルタイム接続を利用する", "Use the realtime connection"),
    },
    Domain {
        name: "dev",
        sensitive: false,
        read: ("Webhook・配信ログ・API利用状況を見る", "See your webhooks, their delivery log and API usage"),
        write: (
            "Webhookを作成・変更・削除する（トークンやアプリは作成できません）",
            "Create, change and delete webhooks (never tokens or applications)",
        ),
    },
];

fn info(domain: &Domain, write: bool) -> ScopeInfo {
    let (ja, en) = if write { domain.write } else { domain.read };
    ScopeInfo {
        scope: format!("{}:{}", domain.name, if write { "write" } else { "read" }),
        domain: domain.name.to_owned(),
        write,
        description: LocalizedText { ja: ja.to_owned(), en: en.to_owned(), ..Default::default() }.into(),
        sensitive: domain.sensitive,
        ..Default::default()
    }
}

/// Every scope, in catalogue order (sensitive domains first, read before write).
pub fn catalogue() -> Vec<ScopeInfo> {
    DOMAINS.iter().flat_map(|domain| [info(domain, false), info(domain, true)]).collect()
}

/// The catalogue entry of one scope.
pub fn find(scope: &str) -> Option<ScopeInfo> {
    let (domain, level) = scope.split_once(':')?;
    let domain = DOMAINS.iter().find(|d| d.name == domain)?;
    match level {
        "read" => Some(info(domain, false)),
        "write" => Some(info(domain, true)),
        _ => None,
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
        assert_eq!(all.len(), DOMAINS.len() * 2);
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
            Some(Access::Scope("geo:read".into()))
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
}
