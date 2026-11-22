//! Every piece of user-facing text this domain writes (notification titles
//! and bodies, activity summaries, calendar and profile labels), in Japanese
//! and English, in one place.
use chrono_tz::Tz;
use tokuly_core::{outbound::Localized, prelude::*};
use tokuly_proto::plans::{FreeWindow, Rsvp};

use crate::{dates, milestones::Mark};

/// Title and body of a notification.
pub(crate) struct Notice {
    pub title: Localized,
    pub body: Localized,
}

fn notice(title: (String, String), body: (String, String)) -> Notice {
    Notice { title: Localized::new(title.0, title.1), body: Localized::new(body.0, body.1) }
}

fn both(ja: &str, en: &str) -> (String, String) {
    (ja.to_owned(), en.to_owned())
}

/// A plan as texts name it: its emoji, when it has one, and its title.
pub(crate) fn plan_name(emoji: &str, title: &str) -> String {
    if emoji.is_empty() { title.to_owned() } else { format!("{emoji} {title}") }
}

/// "11/24(火) 18:00" / "Tue, Nov 24 18:00": an instant on the clocks of `zone`.
pub(crate) fn time_text(at: Time, zone: Tz) -> (String, String) {
    let local = at.with_timezone(&zone);
    let (ja, en) = dates::day_text(local.date_naive());
    let clock = local.format("%H:%M");
    (format!("{ja} {clock}"), format!("{en} {clock}"))
}

// ───────────────────────────── ひま ─────────────────────────────

/// "今ひま" / "Free now": how a status is announced. A custom window that
/// has not started yet says from when.
pub(crate) fn free_window(window: FreeWindow, starts_at: Time, now: Time, zone: Tz) -> (String, String) {
    match window {
        FreeWindow::FREE_WINDOW_TODAY => both("今日ひま", "Free today"),
        FreeWindow::FREE_WINDOW_TONIGHT => both("今夜ひま", "Free tonight"),
        FreeWindow::FREE_WINDOW_WEEKEND => both("今週末ひま", "Free this weekend"),
        FreeWindow::FREE_WINDOW_CUSTOM if starts_at > now => {
            let (ja, en) = time_text(starts_at, zone);
            (format!("{ja}からひま"), format!("Free from {en}"))
        }
        _ => both("今ひま", "Free now"),
    }
}

/// A status as one line: the window and, when there is one, the note.
pub(crate) fn free_line(window: (String, String), note: &str) -> (String, String) {
    if note.is_empty() {
        (format!("{}!", window.0), format!("{}!", window.1))
    } else {
        (format!("{}!「{note}」", window.0), format!("{}: “{note}”", window.1))
    }
}

pub(crate) fn friend_free(name: &str, line: (String, String)) -> Notice {
    notice((format!("{name}さんがひましてるよ!"), format!("{name} is free!")), line)
}

pub(crate) fn nudge(from: &str, message: &str) -> Notice {
    let body = if message.is_empty() {
        both("ひまなら一緒にあそぼう!", "You're free? Let's hang out!")
    } else {
        (message.to_owned(), message.to_owned())
    };
    notice((format!("{from}さんから「あそぼ!」"), format!("{from}: “Let's hang out!”")), body)
}

/// Title of the plan suggested when someone answers "あそぼ!".
pub(crate) const HANG_OUT_TITLE: &str = "あそぼ!";

// ───────────────────────────── plans ─────────────────────────────

pub(crate) const TIME_UNDECIDED: (&str, &str) = ("日時未定", "time to be decided");

pub(crate) fn invited(by: &str, plan: &str, when: &(String, String)) -> Notice {
    notice(
        (format!("{by}さんから予定のお誘い!"), format!("{by} invited you!")),
        (format!("「{plan}」{}", when.0), format!("“{plan}” — {}", when.1)),
    )
}

pub(crate) fn plan_updated(plan: &str, time_changed: bool, when: &(String, String), place: Option<&str>) -> Notice {
    let body = match (time_changed, place) {
        (true, Some(place)) => (
            format!("「{plan}」が{}・{place}に変更になったよ!", when.0),
            format!("“{plan}” is now {} at {place}", when.1),
        ),
        (true, None) => (format!("「{plan}」が{}に変更になったよ!", when.0), format!("“{plan}” is now {}", when.1)),
        (false, Some(place)) => {
            (format!("「{plan}」の場所が{place}に変更になったよ!"), format!("“{plan}” moved to {place}"))
        }
        (false, None) => (format!("「{plan}」の場所が変更になったよ!"), format!("The place of “{plan}” changed")),
    };
    notice(both("予定が変更されたよ!", "A plan changed"), body)
}

pub(crate) fn plan_cancelled(plan: &str, reason: &str) -> Notice {
    let body = if reason.is_empty() {
        (format!("「{plan}」が中止になったよ"), format!("“{plan}” was cancelled"))
    } else {
        (format!("「{plan}」が中止になったよ:{reason}"), format!("“{plan}” was cancelled: {reason}"))
    };
    notice(both("予定が中止になったよ", "A plan was cancelled"), body)
}

pub(crate) fn rsvp(from: &str, answer: Rsvp, plan: &str, comment: &str) -> Notice {
    let (ja, en) = match answer {
        Rsvp::RSVP_GOING => ("参加", "is going"),
        Rsvp::RSVP_MAYBE => ("未定", "might come"),
        Rsvp::RSVP_WAITLISTED => ("キャンセル待ち", "is on the waitlist"),
        _ => ("不参加", "can't make it"),
    };
    let body = if comment.is_empty() {
        (format!("「{plan}」"), format!("“{plan}”"))
    } else {
        (format!("「{plan}」:{comment}"), format!("“{plan}”: {comment}"))
    };
    notice((format!("{from}さんが「{ja}」と回答!"), format!("{from} {en}")), body)
}

pub(crate) fn reminder_day_before(plan: &str, when: &(String, String)) -> Notice {
    notice(
        (format!("明日は「{plan}」!"), format!("Tomorrow: “{plan}”")),
        (format!("{}だよ! 準備はOK?", when.0), format!("{} — all set?", when.1)),
    )
}

pub(crate) fn reminder_soon(plan: &str, minutes: i32, place: &str) -> Notice {
    let (ja, en) = match minutes {
        m if m >= 120 && m % 60 == 0 => (format!("あと{}時間", m / 60), format!("in {} hours", m / 60)),
        60 => ("あと1時間".to_owned(), "in 1 hour".to_owned()),
        m => (format!("あと{m}分"), format!("in {m} minutes")),
    };
    let body = if place.is_empty() {
        (format!("{ja}で始まるよ!"), format!("Starts {en}"))
    } else {
        (format!("{ja}で始まるよ! 場所:{place}"), format!("Starts {en} at {place}"))
    };
    notice((format!("もうすぐ「{plan}」!"), format!("Soon: “{plan}”")), body)
}

pub(crate) fn promoted(plan: &str) -> Notice {
    notice(
        both("参加できるようになったよ!", "You're in!"),
        (
            format!("「{plan}」に空きが出て、キャンセル待ちから繰り上がったよ!"),
            format!("A place opened up in “{plan}” and you moved up from the waitlist"),
        ),
    )
}

pub(crate) fn settled(by: &str, money: &str, plan: &str) -> Notice {
    notice(
        both("割り勘の精算", "Split settled"),
        (
            format!("{by}さんが{money}を受け取ったと記録したよ(「{plan}」)"),
            format!("{by} recorded receiving {money} from you (“{plan}”)"),
        ),
    )
}

/// To the payer of a transfer that the receiver took back.
pub(crate) fn settlement_removed(by: &str, money: &str, plan: &str) -> Notice {
    notice(
        both("精算が取り消されたよ", "A settlement was taken back"),
        (
            format!("{by}さんが{money}の受け取りを取り消したよ(「{plan}」)"),
            format!("{by} took back the record of receiving {money} from you (“{plan}”)"),
        ),
    )
}

pub(crate) fn plan_created_summary(plan: &str) -> Localized {
    Localized::new(format!("「{plan}」の予定を立てました"), format!("Planned “{plan}”"))
}

pub(crate) fn plan_attended_summary(plan: &str) -> Localized {
    Localized::new(format!("「{plan}」に参加しました"), format!("Went to “{plan}”"))
}

// ───────────────────────────── date polls ─────────────────────────────

pub(crate) fn poll_invited(by: &str, poll: &str) -> Notice {
    notice(
        (format!("{by}さんから日程調整のお願い!"), format!("{by} asks when you're free")),
        (format!("「{poll}」行ける日を教えてね!"), format!("“{poll}” — tell them which days work")),
    )
}

pub(crate) fn poll_answered(by: &str, poll: &str) -> Notice {
    notice(
        (format!("{by}さんが日程に回答したよ!"), format!("{by} answered your poll")),
        (format!("「{poll}」"), format!("“{poll}”")),
    )
}

pub(crate) fn poll_reminder(poll: &str) -> Notice {
    notice(
        both("日程調整の回答がまだだよ!", "A date poll is waiting for you"),
        (format!("「{poll}」行ける日を教えてね!"), format!("“{poll}” — tell them which days work")),
    )
}

pub(crate) fn poll_closed(poll: &str) -> Notice {
    notice(
        both("日程調整をしめきったよ!", "Your date poll closed"),
        (format!("「{poll}」日程を決めよう!"), format!("“{poll}” — pick the date!")),
    )
}

pub(crate) const POLL_DEADLINE_LABEL: (&str, &str) = ("回答しめきり", "Answers due");

// ───────────────────── countdowns and anniversaries ─────────────────────

pub(crate) fn countdown_today(countdown: &str) -> Notice {
    notice(
        (format!("今日は「{countdown}」!"), format!("Today: “{countdown}”")),
        both("ついにこの日が来たよ!", "The day is finally here!"),
    )
}

/// "あと12日" / "12 days to go", "今日!" on the day, "3日前" once it has passed.
pub(crate) fn days_left(days: i32) -> (String, String) {
    match days {
        0 => both("今日!", "Today!"),
        1 => both("あと1日", "1 day to go"),
        d if d > 0 => (format!("あと{d}日"), format!("{d} days to go")),
        -1 => both("1日前", "1 day ago"),
        d => (format!("{}日前", -d), format!("{} days ago", -d)),
    }
}

pub(crate) fn counter_started(by: &str) -> Notice {
    notice(
        both("友達記念日カウンター!", "Friendship counter"),
        (
            format!("{by}さんが友達になってからの日数を数えはじめたよ!"),
            format!("{by} started counting the days you've been friends"),
        ),
    )
}

pub(crate) fn couple_invite(by: &str) -> Notice {
    notice(
        both("カップル記念日のお誘い!", "Couple counter invitation"),
        (
            format!("{by}さんからカップル記念日カウンターのお誘いが届いたよ!"),
            format!("{by} invited you to a couple counter"),
        ),
    )
}

pub(crate) fn couple_accepted(by: &str) -> Notice {
    notice(
        both("カップル記念日スタート!", "Couple counter started"),
        (format!("{by}さんがお誘いを受けたよ! 🎉"), format!("{by} accepted your invitation 🎉")),
    )
}

/// "ゆいさんと友達になって100日記念日!" — a milestone of a counter with `partner`.
pub(crate) fn milestone(partner: &str, mark: Mark, couple: bool) -> Notice {
    let label = mark.label();
    let body = if couple {
        (
            format!("{partner}さんと付き合って{}! おめでとう 🎉", label.ja),
            format!("{} together with {partner}. Congratulations 🎉", label.en),
        )
    } else {
        (
            format!("{partner}さんと友達になって{}! 🎉", label.ja),
            format!("{} of friendship with {partner} 🎉", label.en),
        )
    };
    notice((format!("今日は{}!", label.ja), format!("Today: {}", label.en)), body)
}

/// Calendar label of a counter's milestone: "ゆいさんと100日記念日".
pub(crate) fn milestone_label(partner: &str, mark: Mark) -> Localized {
    let label = mark.label();
    Localized::new(format!("{partner}さんと{}", label.ja), format!("{} with {partner}", label.en))
}

pub(crate) fn birthday_label(name: &str) -> Localized {
    Localized::new(format!("{name}さんの誕生日"), format!("{name}'s birthday"))
}

pub(crate) const OWN_BIRTHDAY_LABEL: (&str, &str) = ("自分の誕生日", "Your birthday");

/// Text of the couple card on a profile: "ゆいさんと123日目".
pub(crate) fn couple_days(partner: &str, days: i64, count_start_day: bool) -> Localized {
    if count_start_day {
        Localized::new(format!("{partner}さんと{days}日目"), format!("Day {days} with {partner}"))
    } else {
        Localized::new(format!("{partner}さんと{days}日"), format!("{days} days with {partner}"))
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    #[test]
    fn windows_are_announced_by_their_preset() {
        let zone = chrono_tz::Asia::Tokyo;
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 22, 3, 0, 0).unwrap();
        assert_eq!(free_window(FreeWindow::FREE_WINDOW_NOW, now, now, zone).0, "今ひま");
        assert_eq!(free_window(FreeWindow::FREE_WINDOW_WEEKEND, now, now, zone).1, "Free this weekend");
        // A custom window that already runs is simply "now"; one in the future names its start.
        assert_eq!(free_window(FreeWindow::FREE_WINDOW_CUSTOM, now, now, zone).0, "今ひま");
        let later = chrono::Utc.with_ymd_and_hms(2026, 11, 24, 9, 0, 0).unwrap();
        assert_eq!(
            free_window(FreeWindow::FREE_WINDOW_CUSTOM, later, now, zone),
            ("11/24(火) 18:00からひま".to_owned(), "Free from Tue, Nov 24 18:00".to_owned())
        );
        assert_eq!(free_line(both("今ひま", "Free now"), "").0, "今ひま!");
        assert_eq!(
            free_line(both("今ひま", "Free now"), "カラオケ行きたい"),
            ("今ひま!「カラオケ行きたい」".to_owned(), "Free now: “カラオケ行きたい”".to_owned())
        );
    }

    #[test]
    fn reminders_say_how_long_is_left() {
        assert_eq!(reminder_soon("花火", 60, "").body.ja, "あと1時間で始まるよ!");
        assert_eq!(reminder_soon("花火", 30, "駅前").body.ja, "あと30分で始まるよ! 場所:駅前");
        assert_eq!(reminder_soon("花火", 180, "").body.en, "Starts in 3 hours");
        assert_eq!(reminder_soon("花火", 90, "").body.en, "Starts in 90 minutes");
    }

    #[test]
    fn days_left_reads_naturally_on_both_sides_of_the_day() {
        assert_eq!(days_left(12).0, "あと12日");
        assert_eq!(days_left(1).1, "1 day to go");
        assert_eq!(days_left(0).0, "今日!");
        assert_eq!(days_left(-3), ("3日前".to_owned(), "3 days ago".to_owned()));
    }

    #[test]
    fn plans_are_named_with_their_emoji() {
        assert_eq!(plan_name("🎤", "カラオケ"), "🎤 カラオケ");
        assert_eq!(plan_name("", "カラオケ"), "カラオケ");
    }
}
