//! Every piece of user-facing text this domain writes (notification titles
//! and bodies, activity summaries), in Japanese and English, in one place.
use tokuly_core::outbound::Localized;
use tokuly_proto::social::{FriendSource, MapReactionKind};

/// Title and body of a notification.
pub(crate) struct Notice {
    pub title: Localized,
    pub body: Localized,
}

fn notice(title: (&str, &str), body: (String, String)) -> Notice {
    Notice { title: Localized::new(title.0, title.1), body: Localized::new(body.0, body.1) }
}

/// "X wants to be friends", worded after how the request came about.
pub(crate) fn friend_request(from: &str, source: FriendSource) -> Notice {
    let body = match source {
        FriendSource::FRIEND_SOURCE_INVITE_LINK => {
            (format!("{from}さんがあなたの招待でTokulyに来たよ! 友達になろう!"), format!("{from} joined Tokuly through your invite. Become friends!"))
        }
        FriendSource::FRIEND_SOURCE_BUMP => (format!("{from}さんとBumpしたよ! 友達になろう!"), format!("You bumped with {from}. Become friends!")),
        _ => (format!("{from}さんから友達リクエストが届いたよ!"), format!("{from} wants to be friends!")),
    };
    notice(("友達リクエスト!", "Friend request"), body)
}

pub(crate) fn friend_accepted(by: &str) -> Notice {
    notice(("友達になったよ!", "New friend!"), (format!("{by}さんと友達になったよ!"), format!("You and {by} are friends now!")))
}

pub(crate) fn bumped(with: &str) -> Notice {
    notice(("Bump!", "Bump!"), (format!("{with}さんとBumpしたよ!"), format!("You bumped with {with}!")))
}

pub(crate) fn friends_bumped(a: &str, b: &str) -> Notice {
    notice(("Bump!", "Bump!"), (format!("{a}さんと{b}さんがBumpしたよ!"), format!("{a} and {b} bumped!")))
}

pub(crate) fn buzz(from: &str) -> Notice {
    notice(("ブルブル!", "Buzz!"), (format!("{from}さんがすぐ近くで呼んでるよ!"), format!("{from} is buzzing you from nearby!")))
}

pub(crate) fn knock(from: &str) -> Notice {
    notice(("コンコン!", "Knock knock!"), (format!("{from}さんがノックしてるよ!"), format!("{from} is knocking!")))
}

pub(crate) fn map_reaction(from: &str, kind: MapReactionKind, emoji: &str) -> Notice {
    let body = match kind {
        MapReactionKind::MAP_REACTION_KIND_EYES => (format!("{from}さんがあなたを見てるよ 👀"), format!("{from} is checking on you 👀")),
        MapReactionKind::MAP_REACTION_KIND_DRINKS => (format!("{from}さんが「遊ぼう!」って言ってるよ 🍻"), format!("{from} wants to hang out 🍻")),
        MapReactionKind::MAP_REACTION_KIND_EMOJI => (format!("{from}さんから{emoji}が届いたよ!"), format!("{from} sent you {emoji}")),
        MapReactionKind::MAP_REACTION_KIND_STAMP => (format!("{from}さんからスタンプが届いたよ!"), format!("{from} threw a stamp at you!")),
        MapReactionKind::MAP_REACTION_KIND_HEARTS | MapReactionKind::MAP_REACTION_KIND_UNSPECIFIED => {
            (format!("{from}さんからハートが届いたよ! 💖"), format!("{from} sent you hearts 💖"))
        }
    };
    notice(("マップリアクション", "Map reaction"), body)
}

/// Activity log summary: the user became friends with `with`.
pub(crate) fn became_friends(with: &str) -> Localized {
    Localized::new(format!("{with}さんと友達になったよ!"), format!("Became friends with {with}"))
}

/// Activity log summary: the user bumped with `with`.
pub(crate) fn bumped_activity(with: &str) -> Localized {
    Localized::new(format!("{with}さんとBumpしたよ!"), format!("Bumped with {with}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_wording_follows_the_source() {
        let plain = friend_request("ありす", FriendSource::FRIEND_SOURCE_HANDLE);
        assert_eq!(plain.body.ja, "ありすさんから友達リクエストが届いたよ!");
        assert!(friend_request("ありす", FriendSource::FRIEND_SOURCE_INVITE_LINK).body.ja.contains("招待"));
        assert!(friend_request("Alice", FriendSource::FRIEND_SOURCE_BUMP).body.en.contains("bumped"));
    }

    #[test]
    fn emoji_reactions_show_the_emoji() {
        let text = map_reaction("ぼぶ", MapReactionKind::MAP_REACTION_KIND_EMOJI, "🍜");
        assert_eq!(text.body.ja, "ぼぶさんから🍜が届いたよ!");
        assert_eq!(text.body.en, "ぼぶ sent you 🍜");
    }
}
