//! Preference categories.
//!
//! Users do not toggle individual notification kinds; they toggle a handful
//! of categories. A kind's category comes from the part before its first dot
//! (`chat.message` → `chat`), so a domain that invents a new kind needs no
//! change here. Prefixes nobody listed fall into `system`, which keeps them
//! visible rather than silently dropped.
use tokuly_core::outbound::Localized;

pub struct Category {
    /// Stable ID stored in the database and used by clients.
    pub id: &'static str,
    label: (&'static str, &'static str),
    description: (&'static str, &'static str),
    /// Kind prefixes that belong to this category.
    prefixes: &'static [&'static str],
    /// The inbox toggle cannot be turned off.
    pub inbox_locked: bool,
    /// Pushes of this category are worth waking a sleeping device for
    /// (someone is waiting, or it is about the user's safety); the others
    /// may be delivered when the device next has a good moment. This is only
    /// about delivery speed: it does not get past quiet hours or the pause,
    /// which only a request marked `urgent` does.
    pub high_priority: bool,
}

impl Category {
    pub fn label(&self) -> Localized {
        Localized::new(self.label.0, self.label.1)
    }

    pub fn description(&self) -> Localized {
        Localized::new(self.description.0, self.description.1)
    }
}

/// Every category, in the order settings screens show them.
pub const CATEGORIES: &[Category] = &[
    Category {
        id: "social",
        label: ("友達", "Friends"),
        description: (
            "友達リクエスト、バンプ、ブザー、マップのリアクション",
            "Friend requests, bumps, buzzes and map reactions",
        ),
        prefixes: &["social", "friend"],
        inbox_locked: false,
        high_priority: true,
    },
    Category {
        id: "chat",
        label: ("チャット・通話", "Chat & calls"),
        description: (
            "メッセージ、通話、POVリクエスト",
            "Messages, calls and POV requests",
        ),
        prefixes: &["chat", "call"],
        inbox_locked: false,
        high_priority: true,
    },
    Category {
        id: "geo",
        label: ("マップ", "Map"),
        description: (
            "友達の到着・出発、集合、SOS",
            "Friends arriving and leaving, gatherings and SOS",
        ),
        prefixes: &["geo", "map", "place"],
        inbox_locked: false,
        high_priority: true,
    },
    Category {
        id: "notes",
        label: ("ノート・記憶", "Notes & stories"),
        description: (
            "返信、リノート、記憶へのリアクション",
            "Replies, renotes and reactions to your stories",
        ),
        prefixes: &["notes", "note", "story", "stories"],
        inbox_locked: false,
        high_priority: false,
    },
    Category {
        id: "video",
        label: ("動画", "Videos"),
        description: (
            "登録チャンネルの新着動画、ウォッチパーティー",
            "New videos from your subscriptions and watch parties",
        ),
        prefixes: &["video", "channel", "playlist"],
        inbox_locked: false,
        high_priority: false,
    },
    Category {
        id: "live",
        label: ("ライブ配信", "Live"),
        description: (
            "配信開始、リマインダー、ギフト",
            "Streams going live, reminders and gifts",
        ),
        prefixes: &["live"],
        inbox_locked: false,
        high_priority: true,
    },
    Category {
        id: "shorts",
        label: ("ショート", "Shorts"),
        description: (
            "ショートへの反応と新着",
            "Reactions to your shorts and new ones to watch",
        ),
        prefixes: &["shorts", "short"],
        inbox_locked: false,
        high_priority: false,
    },
    Category {
        id: "engagement",
        label: ("コメント・リアクション", "Comments & reactions"),
        description: (
            "コメント、いいね、メンション",
            "Comments, likes and mentions",
        ),
        prefixes: &["engagement", "comment", "reaction"],
        inbox_locked: false,
        high_priority: false,
    },
    Category {
        id: "stamp",
        label: ("スタンプ", "Stamps"),
        description: (
            "スタンプパックの審査結果や売れ行き",
            "Stamp pack reviews and sales",
        ),
        prefixes: &["stamp", "stamps"],
        inbox_locked: false,
        high_priority: false,
    },
    Category {
        id: "wallet",
        label: ("コイン", "Coins"),
        description: (
            "コインの受け取り、購入、出金",
            "Coins received, purchases and payouts",
        ),
        prefixes: &["wallet"],
        inbox_locked: false,
        high_priority: false,
    },
    Category {
        id: "feed",
        label: ("ニュース・お知らせ", "News & updates"),
        description: (
            "ニュースとアップデートのお知らせ",
            "News picks and product updates",
        ),
        prefixes: &["feed", "news", "announcement"],
        inbox_locked: false,
        high_priority: false,
    },
    Category {
        id: SYSTEM,
        label: ("アカウント・セキュリティ", "Account & security"),
        description: (
            "ログイン通知や大事なお知らせ",
            "Sign-in alerts and important notices",
        ),
        prefixes: &[
            "system", "identity", "account", "security", "admin", "media", "search",
        ],
        inbox_locked: true,
        high_priority: true,
    },
];

const SYSTEM: &str = "system";

/// The category with this ID.
pub fn find(id: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.id == id)
}

/// The category a notification kind belongs to.
pub fn of_kind(kind: &str) -> &'static Category {
    let prefix = kind.split('.').next().unwrap_or_default();
    CATEGORIES
        .iter()
        .find(|c| c.prefixes.contains(&prefix))
        .unwrap_or_else(|| find(SYSTEM).expect("the system category exists"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_map_to_categories_by_prefix() {
        assert_eq!(of_kind("chat.message").id, "chat");
        assert_eq!(of_kind("social.friend_request").id, "social");
        assert_eq!(of_kind("note.reply").id, "notes");
        assert_eq!(of_kind("live.started").id, "live");
        assert_eq!(of_kind("wallet.gift_received").id, "wallet");
        assert_eq!(of_kind("identity.new_sign_in").id, "system");
    }

    #[test]
    fn unknown_prefixes_fall_back_to_system() {
        assert_eq!(of_kind("mystery.thing").id, "system");
        assert_eq!(of_kind("").id, "system");
        assert_eq!(
            of_kind("chatty.message").id,
            "system",
            "prefixes match whole segments"
        );
    }

    #[test]
    fn ids_and_prefixes_are_unique() {
        let mut ids: Vec<_> = CATEGORIES.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATEGORIES.len());
        let mut prefixes: Vec<_> = CATEGORIES
            .iter()
            .flat_map(|c| c.prefixes.iter().copied())
            .collect();
        let total = prefixes.len();
        prefixes.sort_unstable();
        prefixes.dedup();
        assert_eq!(
            prefixes.len(),
            total,
            "a prefix belongs to exactly one category"
        );
        for category in CATEGORIES {
            assert!(
                category.prefixes.contains(&category.id),
                "{} lists its own ID as a prefix",
                category.id
            );
        }
    }

    #[test]
    fn high_priority_categories_are_the_time_critical_ones() {
        let high: Vec<_> = CATEGORIES
            .iter()
            .filter(|c| c.high_priority)
            .map(|c| c.id)
            .collect();
        assert_eq!(high, ["social", "chat", "geo", "live", "system"]);
    }

    #[test]
    fn only_system_is_locked() {
        let locked: Vec<_> = CATEGORIES
            .iter()
            .filter(|c| c.inbox_locked)
            .map(|c| c.id)
            .collect();
        assert_eq!(locked, ["system"]);
    }
}
