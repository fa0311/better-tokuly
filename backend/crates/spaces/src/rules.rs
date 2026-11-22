//! The rules of the domain that need no database and no LiveKit: what is a
//! valid title or emoji, what a role may do, who may see a room, when a
//! room is over, and when someone who dropped off counts as gone.
use chrono::Duration;
use tokuly_core::prelude::*;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::model::{audience, end, leave, role};

pub const MAX_TITLE_CHARS: usize = 80;
pub const MAX_TAGS: usize = 5;
pub const MAX_TAG_CHARS: usize = 30;
pub const MAX_HANGOUT_NAME_CHARS: usize = 40;
pub const MAX_PIN_CHARS: usize = 200;
pub const MAX_PIN_URL_BYTES: usize = 2000;
/// A scheduled room starts at least this far in the future.
pub const MIN_SCHEDULE_LEAD_SECONDS: i64 = 60;
/// The emoji of a hangout whose owner did not pick one.
pub const DEFAULT_HANGOUT_EMOJI: &str = "🛋️";

// ───────────────────────────── text ─────────────────────────────

fn has_control_chars(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// A room title: 1–80 characters on one line.
pub fn title(input: &str) -> AppResult<String> {
    let title = input.trim();
    let length = title.chars().count();
    ensure!(
        (1..=MAX_TITLE_CHARS).contains(&length) && !has_control_chars(title),
        invalid,
        "INVALID_TITLE",
        "a title has 1 to {MAX_TITLE_CHARS} characters on one line"
    );
    Ok(title.to_owned())
}

/// One tag as it is stored and compared: without '#', lowercase. `None`
/// when nothing usable is left.
pub fn tag(input: &str) -> Option<String> {
    let tag = input.trim().trim_start_matches(['#', '＃']).to_lowercase();
    let usable = (1..=MAX_TAG_CHARS).contains(&tag.chars().count())
        && !tag.chars().any(|c| c.is_whitespace() || c.is_control() || c == '#' || c == '＃');
    usable.then_some(tag)
}

/// The tags of a room: up to five, each up to 30 characters without
/// spaces; duplicates collapse.
pub fn tags(input: &[String]) -> AppResult<Vec<String>> {
    let mut tags: Vec<String> = Vec::new();
    for raw in input {
        if raw.trim().is_empty() {
            continue;
        }
        let Some(tag) = tag(raw) else {
            bail!(invalid, "INVALID_TAGS", "a tag has 1 to {MAX_TAG_CHARS} characters and no spaces");
        };
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    ensure!(tags.len() <= MAX_TAGS, invalid, "INVALID_TAGS", "a room has at most {MAX_TAGS} tags");
    Ok(tags)
}

/// The language a room is held in: the primary subtag of a BCP 47 tag
/// (`ja`, `en-US` → `en`), or empty when the host did not say.
pub fn language(input: &str) -> AppResult<String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(String::new());
    }
    let primary = input.split(['-', '_']).next().unwrap_or_default().to_ascii_lowercase();
    ensure!(
        (2..=3).contains(&primary.len()) && primary.bytes().all(|b| b.is_ascii_lowercase()),
        invalid,
        "INVALID_LANGUAGE",
        "a language is a BCP 47 tag such as \"ja\" or \"en\""
    );
    Ok(primary)
}

/// Whether a character belongs to an emoji (pictographs, symbols,
/// regional indicators and the marks that combine them).
fn is_emoji_char(c: char) -> bool {
    matches!(
        c as u32,
        0x00A9 | 0x00AE            // © ®
        | 0x200D                   // zero-width joiner
        | 0x203C..=0x2BFF          // arrows, technical, enclosed, geometric, miscellaneous symbols, dingbats
        | 0x3030 | 0x303D | 0x3297 | 0x3299
        | 0xFE0F                   // emoji presentation
        | 0x1F000..=0x1FAFF        // pictographs, emoticons, transport, flags
        | 0xE0020..=0xE007F        // tag characters (subdivision flags)
    )
}

/// An emoji as people send it: one emoji (a single grapheme, so a flag or
/// a family counts as one), or a custom emoji as `:shortcode:`.
pub fn emoji(input: &str) -> AppResult<String> {
    let emoji = input.trim();
    let custom = emoji.len() >= 3
        && emoji.len() <= 34
        && emoji.starts_with(':')
        && emoji.ends_with(':')
        && emoji[1..emoji.len() - 1].bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    let single = emoji.len() <= 64
        && emoji.graphemes(true).count() == 1
        && emoji.chars().any(is_emoji_char)
        // Keycaps are a digit with marks; everything else is emoji through and through.
        && emoji.chars().all(|c| is_emoji_char(c) || c.is_ascii_digit() || matches!(c, '#' | '*'));
    ensure!(custom || single, invalid, "INVALID_EMOJI", "send one emoji, or a custom emoji as :shortcode:");
    Ok(emoji.to_owned())
}

/// A hangout name: 1–40 characters on one line.
pub fn hangout_name(input: &str) -> AppResult<String> {
    let name = input.trim();
    let length = name.chars().count();
    ensure!(
        (1..=MAX_HANGOUT_NAME_CHARS).contains(&length) && !has_control_chars(name),
        invalid,
        "INVALID_NAME",
        "a hangout name has 1 to {MAX_HANGOUT_NAME_CHARS} characters on one line"
    );
    Ok(name.to_owned())
}

/// A pinned note and link. Both empty unpins.
pub fn pin(text: &str, url: &str) -> AppResult<(String, String)> {
    let text = text.trim();
    let url = url.trim();
    ensure!(
        text.chars().count() <= MAX_PIN_CHARS && !text.chars().any(|c| c.is_control() && c != '\n'),
        invalid,
        "INVALID_PIN",
        "a pinned note has at most {MAX_PIN_CHARS} characters"
    );
    if !url.is_empty() {
        let parsed = url::Url::parse(url).ok().filter(|parsed| {
            matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some_and(|host| !host.is_empty())
        });
        ensure!(
            parsed.is_some() && url.len() <= MAX_PIN_URL_BYTES,
            invalid,
            "INVALID_PIN",
            "a pinned link is an http(s) URL of at most {MAX_PIN_URL_BYTES} characters"
        );
    }
    Ok((text.to_owned(), url.to_owned()))
}

/// The planned start of a room: at least a minute and at most `max_days`
/// from `now`.
pub fn schedule(at: Time, now: Time, max_days: i64) -> AppResult<Time> {
    ensure!(
        at >= now + Duration::seconds(MIN_SCHEDULE_LEAD_SECONDS),
        invalid,
        "INVALID_SCHEDULE",
        "a room is scheduled at least a minute ahead"
    );
    ensure!(
        at <= now + Duration::days(max_days),
        invalid,
        "INVALID_SCHEDULE",
        "a room can be scheduled at most {max_days} days ahead"
    );
    Ok(at)
}

// ───────────────────────────── roles ─────────────────────────────

/// On stage: may talk.
pub fn on_stage(role: i16) -> bool {
    role >= role::SPEAKER
}

/// Host or co-host.
pub fn is_moderator(role: i16) -> bool {
    role >= role::COHOST
}

/// Whether someone with role `actor` may moderate someone with role
/// `target` (move, mute, remove): the host anyone but themselves, a
/// co-host speakers and listeners.
pub fn may_moderate(actor: i16, target: i16) -> bool {
    match actor {
        role::HOST => target < role::HOST,
        role::COHOST => target < role::COHOST,
        _ => false,
    }
}

/// The role someone keeps for when they come back. The host and co-hosts
/// keep theirs when they leave or drop off; a speaker returns as a
/// listener; whoever is thrown out loses everything. When the room ends,
/// roles stay as they were, for the record.
pub fn role_after_leaving(role: i16, reason: i16) -> i16 {
    match reason {
        leave::ENDED => role,
        leave::LEFT | leave::DISCONNECTED if is_moderator(role) => role,
        _ => role::LISTENER,
    }
}

/// The role as LiveKit participant metadata carries it.
pub fn role_name(role: i16) -> &'static str {
    match role {
        role::HOST => "host",
        role::COHOST => "cohost",
        role::SPEAKER => "speaker",
        _ => "listener",
    }
}

// ───────────────────────────── access ─────────────────────────────

/// What decides whether someone may see (and, while it is live, enter) a
/// room. Everything is false for an anonymous viewer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Access {
    pub audience: i16,
    pub host: bool,
    /// Inside right now.
    pub inside: bool,
    pub invited: bool,
    pub friend_of_host: bool,
    /// The viewer and the host are on either side of a block.
    pub blocked: bool,
    /// The viewer is banned from the room.
    pub banned: bool,
}

impl Access {
    /// A block or a ban hides the room; otherwise public rooms are open,
    /// the host, invited people and those inside always get in, and a
    /// friends room admits the host's friends.
    pub fn may_see(&self) -> bool {
        if self.host {
            return true;
        }
        if self.blocked || self.banned {
            return false;
        }
        self.audience == audience::PUBLIC
            || self.invited
            || self.inside
            || (self.audience == audience::FRIENDS && self.friend_of_host)
    }
}

// ───────────────────────────── when a room is over ─────────────────────────────

/// What becomes of a live room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// It goes on.
    Continue,
    /// The host stayed away too long and a co-host is inside: they take over.
    HandOver,
    /// It ends, for this `SpaceEndReason`.
    End(i16),
}

/// What [`settle`] judges a live room by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    /// How long the host has not been inside; `None` while they are.
    pub host_absent_seconds: Option<i64>,
    /// How long nobody has been inside; `None` while somebody is.
    pub empty_seconds: Option<i64>,
    /// A co-host who could take over is inside.
    pub successor_inside: bool,
}

/// Decides what becomes of a live room: it ends once it has been empty for
/// `empty_grace` seconds; once the host has been away for `host_grace`
/// seconds it is handed to a co-host, or ends when there is none.
pub fn settle(standing: Standing, host_grace: i64, empty_grace: i64) -> Verdict {
    if standing.empty_seconds.is_some_and(|empty| empty >= empty_grace) {
        return Verdict::End(end::EMPTY);
    }
    match standing.host_absent_seconds {
        Some(absent) if absent >= host_grace => {
            if standing.successor_inside {
                Verdict::HandOver
            } else {
                Verdict::End(end::HOST_LEFT)
            }
        }
        _ => Verdict::Continue,
    }
}

// ───────────────────────────── presence ─────────────────────────────

/// What a comparison with LiveKit says about someone marked as inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sighting {
    /// Connected.
    Seen,
    /// Not connected, but within the time allowed to connect or reconnect.
    Awaited,
    /// Was connected and is not any more: the reconnect clock starts now.
    Missing,
    /// Not connected for too long: they are gone.
    Gone,
}

/// Judges one person marked as inside. Someone who never showed up has
/// `connect_grace` seconds from entering; someone who was connected has
/// `reconnect_grace` seconds from the moment they were first found missing.
pub fn sighting(
    connected: bool,
    joined_at: Time,
    seen_at: Option<Time>,
    missing_since: Option<Time>,
    now: Time,
    connect_grace: i64,
    reconnect_grace: i64,
) -> Sighting {
    if connected {
        return Sighting::Seen;
    }
    match (seen_at, missing_since) {
        (None, _) if now - joined_at >= Duration::seconds(connect_grace) => Sighting::Gone,
        (None, _) => Sighting::Awaited,
        (Some(_), None) if reconnect_grace <= 0 => Sighting::Gone,
        (Some(_), None) => Sighting::Missing,
        (Some(_), Some(since)) if now - since >= Duration::seconds(reconnect_grace) => Sighting::Gone,
        (Some(_), Some(_)) => Sighting::Awaited,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason<T: std::fmt::Debug>(result: AppResult<T>) -> String {
        result.unwrap_err().reason.into_owned()
    }

    #[test]
    fn titles_are_trimmed_and_bounded() {
        assert_eq!(title("  放課後トーク  ").unwrap(), "放課後トーク");
        assert_eq!(title(&"あ".repeat(80)).unwrap().chars().count(), 80);
        assert_eq!(reason(title(&"あ".repeat(81))), "INVALID_TITLE");
        assert_eq!(reason(title("   ")), "INVALID_TITLE");
        assert_eq!(reason(title("two\nlines")), "INVALID_TITLE");
    }

    #[test]
    fn tags_are_normalized() {
        let input =
            ["#Music".to_owned(), " 推し活 ".to_owned(), "music".to_owned(), "＃雑談".to_owned(), "".to_owned()];
        assert_eq!(tags(&input).unwrap(), ["music", "推し活", "雑談"]);
        assert_eq!(reason(tags(&["two words".to_owned()])), "INVALID_TAGS");
        assert_eq!(reason(tags(&["a".repeat(31)])), "INVALID_TAGS");
        assert_eq!(reason(tags(&["#".to_owned()])), "INVALID_TAGS");
        let many: Vec<String> = (0..6).map(|n| format!("tag{n}")).collect();
        assert_eq!(reason(tags(&many)), "INVALID_TAGS");
        assert_eq!(tags(&many[..5]).unwrap().len(), 5);
        assert_eq!(tag("#JPOP"), Some("jpop".to_owned()));
        assert_eq!(tag("  "), None);
    }

    #[test]
    fn languages_keep_their_primary_subtag() {
        assert_eq!(language("ja").unwrap(), "ja");
        assert_eq!(language("en-US").unwrap(), "en");
        assert_eq!(language("ZH_hant").unwrap(), "zh");
        assert_eq!(language(" ").unwrap(), "");
        assert_eq!(reason(language("japanese")), "INVALID_LANGUAGE");
        assert_eq!(reason(language("j")), "INVALID_LANGUAGE");
        assert_eq!(reason(language("日本")), "INVALID_LANGUAGE");
    }

    #[test]
    fn one_emoji_at_a_time() {
        for ok in ["🔥", "👍🏽", "🇯🇵", "👨‍👩‍👧", "❤️", "1️⃣", ":tokuly_wave:", "✨"] {
            assert_eq!(emoji(ok).unwrap(), ok, "{ok}");
        }
        for bad in ["", "🔥🔥", "a", "ok", "猫", "1", ":Bad:", "::", ":a b:", "🔥!", " "] {
            assert_eq!(reason(emoji(bad)), "INVALID_EMOJI", "{bad:?}");
        }
        assert!(emoji(DEFAULT_HANGOUT_EMOJI).is_ok());
    }

    #[test]
    fn hangout_names_are_bounded() {
        assert_eq!(hangout_name(" いつメン ").unwrap(), "いつメン");
        assert_eq!(reason(hangout_name("")), "INVALID_NAME");
        assert_eq!(reason(hangout_name(&"あ".repeat(41))), "INVALID_NAME");
    }

    #[test]
    fn pins_take_a_note_and_an_http_link() {
        assert_eq!(
            pin(" 今日のテーマ ", " https://example.com/a?b=c ").unwrap(),
            ("今日のテーマ".into(), "https://example.com/a?b=c".into())
        );
        assert_eq!(pin("", "").unwrap(), (String::new(), String::new()));
        assert!(pin("line one\nline two", "").is_ok());
        assert_eq!(reason(pin(&"あ".repeat(201), "")), "INVALID_PIN");
        for bad in ["javascript:alert(1)", "ftp://example.com", "example.com", "http://", "not a url"] {
            assert_eq!(reason(pin("", bad)), "INVALID_PIN", "{bad}");
        }
        assert_eq!(reason(pin("", &format!("https://example.com/{}", "a".repeat(2000)))), "INVALID_PIN");
    }

    #[test]
    fn schedules_lie_in_the_near_future() {
        let now = now();
        let soon = now + Duration::minutes(5);
        assert_eq!(schedule(soon, now, 30).unwrap(), soon);
        assert_eq!(reason(schedule(now + Duration::seconds(30), now, 30)), "INVALID_SCHEDULE");
        assert_eq!(reason(schedule(now - Duration::hours(1), now, 30)), "INVALID_SCHEDULE");
        assert_eq!(reason(schedule(now + Duration::days(31), now, 30)), "INVALID_SCHEDULE");
    }

    #[test]
    fn roles_form_a_ladder() {
        assert!(!on_stage(role::LISTENER) && on_stage(role::SPEAKER) && on_stage(role::COHOST) && on_stage(role::HOST));
        assert!(!is_moderator(role::SPEAKER) && is_moderator(role::COHOST) && is_moderator(role::HOST));
        // The host moderates everyone else; a co-host only the people below.
        assert!(may_moderate(role::HOST, role::COHOST) && may_moderate(role::HOST, role::LISTENER));
        assert!(!may_moderate(role::HOST, role::HOST));
        assert!(may_moderate(role::COHOST, role::SPEAKER) && may_moderate(role::COHOST, role::LISTENER));
        assert!(!may_moderate(role::COHOST, role::COHOST) && !may_moderate(role::COHOST, role::HOST));
        assert!(!may_moderate(role::SPEAKER, role::LISTENER) && !may_moderate(role::LISTENER, role::LISTENER));
        assert_eq!(
            [role::LISTENER, role::SPEAKER, role::COHOST, role::HOST].map(role_name),
            ["listener", "speaker", "cohost", "host"]
        );
    }

    #[test]
    fn only_moderators_keep_their_role_when_they_go() {
        for reason in [leave::LEFT, leave::DISCONNECTED] {
            assert_eq!(role_after_leaving(role::HOST, reason), role::HOST);
            assert_eq!(role_after_leaving(role::COHOST, reason), role::COHOST);
            assert_eq!(role_after_leaving(role::SPEAKER, reason), role::LISTENER);
            assert_eq!(role_after_leaving(role::LISTENER, reason), role::LISTENER);
        }
        for reason in [leave::REMOVED, leave::BANNED] {
            assert_eq!(role_after_leaving(role::COHOST, reason), role::LISTENER);
            assert_eq!(role_after_leaving(role::SPEAKER, reason), role::LISTENER);
        }
        assert_eq!(role_after_leaving(role::SPEAKER, leave::ENDED), role::SPEAKER, "kept for the record");
    }

    #[test]
    fn who_may_see_a_room() {
        let stranger = |audience| Access { audience, ..Default::default() };
        assert!(stranger(audience::PUBLIC).may_see());
        assert!(!stranger(audience::FRIENDS).may_see());
        assert!(!stranger(audience::INVITED).may_see());

        let friend = |audience| Access { audience, friend_of_host: true, ..Default::default() };
        assert!(friend(audience::FRIENDS).may_see());
        assert!(!friend(audience::INVITED).may_see(), "friendship alone does not open an invited-only room");

        let invited = |audience| Access { audience, invited: true, ..Default::default() };
        assert!(invited(audience::FRIENDS).may_see() && invited(audience::INVITED).may_see());
        assert!(Access { audience: audience::INVITED, inside: true, ..Default::default() }.may_see());

        // A block hides every room of the host, whatever else is true.
        let blocked = Access {
            audience: audience::PUBLIC,
            blocked: true,
            invited: true,
            friend_of_host: true,
            ..Default::default()
        };
        assert!(!blocked.may_see());
        assert!(Access { audience: audience::INVITED, host: true, ..Default::default() }.may_see());
        // So does a ban from the room, public or not.
        assert!(!Access { audience: audience::PUBLIC, banned: true, ..Default::default() }.may_see());
        assert!(
            !Access { audience: audience::FRIENDS, banned: true, friend_of_host: true, ..Default::default() }.may_see()
        );
    }

    #[test]
    fn a_room_outlives_a_short_absence_of_its_host() {
        let here = Standing { host_absent_seconds: None, empty_seconds: None, successor_inside: false };
        assert_eq!(settle(here, 180, 120), Verdict::Continue);
        let away = |seconds, successor_inside| Standing {
            host_absent_seconds: Some(seconds),
            empty_seconds: None,
            successor_inside,
        };
        assert_eq!(settle(away(179, false), 180, 120), Verdict::Continue);
        assert_eq!(settle(away(180, false), 180, 120), Verdict::End(end::HOST_LEFT));
        assert_eq!(settle(away(180, true), 180, 120), Verdict::HandOver);
        assert_eq!(settle(away(5, true), 180, 120), Verdict::Continue, "a co-host does not take over at once");
    }

    #[test]
    fn an_empty_room_ends_first() {
        let empty = |seconds| Standing {
            host_absent_seconds: Some(seconds),
            empty_seconds: Some(seconds),
            successor_inside: false,
        };
        assert_eq!(settle(empty(119), 180, 120), Verdict::Continue);
        assert_eq!(settle(empty(120), 180, 120), Verdict::End(end::EMPTY));
        assert_eq!(settle(empty(500), 180, 120), Verdict::End(end::EMPTY));
        // With a longer empty grace the host's absence decides.
        assert_eq!(settle(empty(200), 180, 600), Verdict::End(end::HOST_LEFT));
    }

    #[test]
    fn dropping_off_is_forgiven_for_a_while() {
        let now = now();
        let ago = |seconds| now - Duration::seconds(seconds);
        // Connected: seen, whatever was recorded before.
        assert_eq!(sighting(true, ago(500), None, Some(ago(400)), now, 45, 20), Sighting::Seen);
        // Never showed up: the time to connect counts from entering.
        assert_eq!(sighting(false, ago(10), None, None, now, 45, 20), Sighting::Awaited);
        assert_eq!(sighting(false, ago(45), None, None, now, 45, 20), Sighting::Gone);
        // Was connected: the reconnect clock starts when they are first missed.
        assert_eq!(sighting(false, ago(500), Some(ago(30)), None, now, 45, 20), Sighting::Missing);
        assert_eq!(sighting(false, ago(500), Some(ago(30)), Some(ago(19)), now, 45, 20), Sighting::Awaited);
        assert_eq!(sighting(false, ago(500), Some(ago(60)), Some(ago(20)), now, 45, 20), Sighting::Gone);
        // No patience configured: gone the moment they are missed.
        assert_eq!(sighting(false, ago(500), Some(ago(1)), None, now, 45, 0), Sighting::Gone);
        assert_eq!(sighting(false, ago(1), None, None, now, 0, 0), Sighting::Gone);
    }
}
