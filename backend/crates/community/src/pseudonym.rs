//! Pseudonyms of anonymous channels.
//!
//! In an anonymous channel a post shows "あおいペンギン #k3f9" instead of its
//! author. The name is a keyed hash (HMAC-SHA256 under a key derived from
//! the server secret) of the thread and the author:
//!
//! - **stable within a thread**: the same author gets the same name on the
//!   thread and on every reply to it, so a conversation can be followed;
//! - **unrelated across threads**: the thread ID is part of the input, so
//!   the same author is someone else in the next thread;
//! - **not reversible**: without the server key the name says nothing about
//!   the account, and trying every member does not work either.
//!
//! Nothing is stored: the name is recomputed whenever a post is rendered.
//! The real author stays in the database for moderation
//! (`CommunityModerationService.RevealAuthor`).
use tokuly_core::{keys::Keys, prelude::*};
use tokuly_proto::{common::LocalizedText, community::Pseudonym};

/// Label of the HMAC key (see [`Keys::mac`]).
const KEY_LABEL: &str = "community/pseudonym/v1";

/// Animals: Japanese name, English name, emoji.
const ANIMALS: &[(&str, &str, &str)] = &[
    ("ペンギン", "Penguin", "🐧"),
    ("パンダ", "Panda", "🐼"),
    ("ねこ", "Cat", "🐱"),
    ("いぬ", "Dog", "🐶"),
    ("うさぎ", "Rabbit", "🐰"),
    ("きつね", "Fox", "🦊"),
    ("たぬき", "Raccoon Dog", "🦝"),
    ("くま", "Bear", "🐻"),
    ("コアラ", "Koala", "🐨"),
    ("とら", "Tiger", "🐯"),
    ("ライオン", "Lion", "🦁"),
    ("ぞう", "Elephant", "🐘"),
    ("きりん", "Giraffe", "🦒"),
    ("しまうま", "Zebra", "🦓"),
    ("かば", "Hippo", "🦛"),
    ("さる", "Monkey", "🐵"),
    ("ひつじ", "Sheep", "🐑"),
    ("やぎ", "Goat", "🐐"),
    ("うし", "Cow", "🐮"),
    ("ぶた", "Pig", "🐷"),
    ("うま", "Horse", "🐴"),
    ("しか", "Deer", "🦌"),
    ("りす", "Squirrel", "🐿️"),
    ("ハムスター", "Hamster", "🐹"),
    ("ねずみ", "Mouse", "🐭"),
    ("はりねずみ", "Hedgehog", "🦔"),
    ("こうもり", "Bat", "🦇"),
    ("ふくろう", "Owl", "🦉"),
    ("ひよこ", "Chick", "🐥"),
    ("にわとり", "Chicken", "🐔"),
    ("あひる", "Duck", "🦆"),
    ("はくちょう", "Swan", "🦢"),
    ("フラミンゴ", "Flamingo", "🦩"),
    ("オウム", "Parrot", "🦜"),
    ("わし", "Eagle", "🦅"),
    ("かえる", "Frog", "🐸"),
    ("かめ", "Turtle", "🐢"),
    ("とかげ", "Lizard", "🦎"),
    ("わに", "Crocodile", "🐊"),
    ("いるか", "Dolphin", "🐬"),
    ("くじら", "Whale", "🐳"),
    ("さめ", "Shark", "🦈"),
    ("たこ", "Octopus", "🐙"),
    ("いか", "Squid", "🦑"),
    ("かに", "Crab", "🦀"),
    ("えび", "Shrimp", "🦐"),
    ("ふぐ", "Blowfish", "🐡"),
    ("あざらし", "Seal", "🦭"),
    ("ラッコ", "Otter", "🦦"),
    ("ナマケモノ", "Sloth", "🦥"),
    ("カンガルー", "Kangaroo", "🦘"),
    ("らくだ", "Camel", "🐫"),
    ("ラマ", "Llama", "🦙"),
    ("ちょうちょ", "Butterfly", "🦋"),
    ("みつばち", "Bee", "🐝"),
    ("てんとうむし", "Ladybug", "🐞"),
    ("かたつむり", "Snail", "🐌"),
    ("ユニコーン", "Unicorn", "🦄"),
    ("ドラゴン", "Dragon", "🐲"),
    ("きょうりゅう", "Dinosaur", "🦕"),
    ("おおかみ", "Wolf", "🐺"),
    ("ゴリラ", "Gorilla", "🦍"),
    ("くじゃく", "Peacock", "🦚"),
    ("ビーバー", "Beaver", "🦫"),
];

/// Colours: Japanese modifier, English adjective, avatar colour.
const COLORS: &[(&str, &str, &str)] = &[
    ("あかい", "Red", "#e5484d"),
    ("あおい", "Blue", "#3b82f6"),
    ("きいろい", "Yellow", "#eab308"),
    ("みどりの", "Green", "#22c55e"),
    ("むらさきの", "Purple", "#8b5cf6"),
    ("ももいろの", "Pink", "#ec4899"),
    ("だいだいの", "Orange", "#f97316"),
    ("みずいろの", "Sky", "#06b6d4"),
    ("ちゃいろの", "Brown", "#a16207"),
    ("はいいろの", "Grey", "#6b7280"),
    ("こんいろの", "Navy", "#1e40af"),
    ("わかくさの", "Lime", "#84cc16"),
    ("あかねの", "Crimson", "#be123c"),
    ("ふじいろの", "Lavender", "#a78bfa"),
    ("あさぎの", "Teal", "#0d9488"),
    ("こがねの", "Gold", "#ca8a04"),
];

/// Characters of the tag: no `l`, `o`, `0` or `1`, which read alike.
const TAG_ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";

/// The name `author_id` goes by in `thread_id`.
pub fn pseudonym(keys: &Keys, thread_id: Uuid, author_id: Uuid) -> Pseudonym {
    let mut input = [0u8; 32];
    input[..16].copy_from_slice(thread_id.as_bytes());
    input[16..].copy_from_slice(author_id.as_bytes());
    let mac = keys.mac(KEY_LABEL, &input);

    let (animal_ja, animal_en, emoji) = ANIMALS[usize::from(u16::from_be_bytes([mac[0], mac[1]])) % ANIMALS.len()];
    let (color_ja, color_en, color) = COLORS[usize::from(mac[2]) % COLORS.len()];
    // 20 bits, five per character.
    let bits = u32::from_be_bytes([0, mac[3], mac[4], mac[5]]);
    let tag: String = (0..4).map(|i| TAG_ALPHABET[((bits >> (15 - 5 * i)) & 31) as usize] as char).collect();

    Pseudonym {
        name: LocalizedText {
            ja: format!("{color_ja}{animal_ja}"),
            en: format!("{color_en} {animal_en}"),
            ..Default::default()
        }
        .into(),
        emoji: emoji.to_owned(),
        color: color.to_owned(),
        tag,
        ..Default::default()
    }
}

/// The pseudonym as one line, for notification texts: "あおいペンギン #k3f9".
pub fn label(pseudonym: &Pseudonym) -> (String, String) {
    let name = pseudonym.name.as_option();
    (
        format!("{} #{}", name.map(|n| n.ja.as_str()).unwrap_or("匿名"), pseudonym.tag),
        format!("{} #{}", name.map(|n| n.en.as_str()).unwrap_or("Anonymous"), pseudonym.tag),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn keys() -> Keys {
        Keys::new("pseudonym-test-secret-pseudonym-test-secret")
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn stable_for_one_author_in_one_thread() {
        let (thread, author) = (id(1), id(2));
        assert_eq!(pseudonym(&keys(), thread, author), pseudonym(&keys(), thread, author));
    }

    #[test]
    fn unrelated_across_threads_and_authors() {
        let keys = keys();
        let author = id(7);
        // One author in many threads: (almost) never the same name twice.
        let names: HashSet<String> = (0..500u128)
            .map(|thread| {
                let p = pseudonym(&keys, id(1000 + thread), author);
                format!("{}{}", p.name.as_option().unwrap().ja, p.tag)
            })
            .collect();
        assert!(names.len() >= 499, "an author must not be recognisable across threads");
        // Many authors in one thread: all told apart.
        let names: HashSet<String> = (0..500u128)
            .map(|author| {
                let p = pseudonym(&keys, id(1), id(5000 + author));
                format!("{}{}", p.name.as_option().unwrap().ja, p.tag)
            })
            .collect();
        assert!(names.len() >= 499, "two authors of one thread must not share a name");
    }

    #[test]
    fn depends_on_the_server_key() {
        let (thread, author) = (id(1), id(2));
        let other = Keys::new("another-secret-another-secret-another-secret");
        assert_ne!(pseudonym(&keys(), thread, author), pseudonym(&other, thread, author));
    }

    #[test]
    fn swapping_thread_and_author_gives_another_name() {
        assert_ne!(pseudonym(&keys(), id(1), id(2)), pseudonym(&keys(), id(2), id(1)));
    }

    #[test]
    fn names_are_well_formed() {
        let p = pseudonym(&keys(), id(3), id(4));
        let name = p.name.as_option().unwrap();
        assert!(!name.ja.is_empty() && !name.en.is_empty() && !p.emoji.is_empty());
        assert!(p.color.starts_with('#') && p.color.len() == 7);
        assert_eq!(p.tag.len(), 4);
        assert!(p.tag.bytes().all(|b| TAG_ALPHABET.contains(&b)));
        let (ja, en) = label(&p);
        assert!(ja.ends_with(&format!("#{}", p.tag)) && en.ends_with(&format!("#{}", p.tag)));
    }

    #[test]
    fn the_word_lists_have_no_duplicates() {
        let animals: HashSet<&str> = ANIMALS.iter().map(|(ja, _, _)| *ja).collect();
        let colors: HashSet<&str> = COLORS.iter().map(|(ja, _, _)| *ja).collect();
        assert_eq!((animals.len(), colors.len()), (ANIMALS.len(), COLORS.len()));
        assert_eq!(TAG_ALPHABET.iter().collect::<HashSet<_>>().len(), 32);
    }
}
