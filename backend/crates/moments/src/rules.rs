//! Everything free of I/O: when a region's prompt goes out, how late a
//! moment is, what a request may contain, and what a locked viewer gets of
//! a picture.
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use rand::{Rng, RngExt as _};
use tokuly_core::prelude::*;
use tokuly_proto::{common::Image, moments::RealmojiEmoji};

/// The longest caption, in characters.
pub const MAX_CAPTION_CHARS: usize = 100;
/// The most retakes an app may report.
pub const MAX_RETAKES: i32 = 99;

/// The date `now` falls on in `region`.
pub fn local_date(region: Tz, now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&region).date_naive()
}

/// `hour` o'clock (0–24) of `date` in `region`. A time that does not exist
/// there (a daylight-saving gap) moves on to the first one that does.
pub fn local_hour(region: Tz, date: NaiveDate, hour: u32) -> DateTime<Utc> {
    let midnight = date.and_time(NaiveTime::MIN);
    let wanted = midnight + Duration::hours(i64::from(hour));
    (0..=180)
        .step_by(15)
        .find_map(|minutes| region.from_local_datetime(&(wanted + Duration::minutes(minutes))).earliest())
        .map_or_else(|| Utc.from_utc_datetime(&wanted), |local| local.with_timezone(&Utc))
}

/// Draws the prompt time of `date` in `region`: uniformly inside the local
/// window `[from_hour, until_hour)`, and not before `not_before` (a region
/// seen for the first time in the middle of its day). `None` when nothing
/// of the window is left.
pub fn draw_prompt<R: Rng + ?Sized>(
    region: Tz,
    date: NaiveDate,
    from_hour: u32,
    until_hour: u32,
    not_before: DateTime<Utc>,
    rng: &mut R,
) -> Option<DateTime<Utc>> {
    let start = local_hour(region, date, from_hour).max(not_before);
    let end = local_hour(region, date, until_hour);
    let span = (end - start).num_seconds();
    (span > 0).then(|| start + Duration::seconds(rng.random_range(0..span)))
}

/// Until when a moment of a day whose prompt went out at `announced_at`
/// counts as on time.
pub fn on_time_until(announced_at: DateTime<Utc>, window_seconds: i64) -> DateTime<Utc> {
    announced_at + Duration::seconds(window_seconds)
}

/// How late a moment posted at `posted_at` is: 0 inside the window (and
/// the upload grace after it), otherwise the seconds since the window
/// closed (at least 1).
pub fn late_seconds(on_time_until: DateTime<Utc>, posted_at: DateTime<Utc>, grace_seconds: i64) -> i64 {
    if posted_at <= on_time_until + Duration::seconds(grace_seconds) {
        0
    } else {
        (posted_at - on_time_until).num_seconds().max(1)
    }
}

/// A caption as stored: trimmed, one line, at most [`MAX_CAPTION_CHARS`].
pub fn caption(input: &str) -> AppResult<String> {
    let caption = input.trim();
    ensure!(
        caption.chars().count() <= MAX_CAPTION_CHARS && !caption.chars().any(char::is_control),
        invalid,
        "INVALID_CAPTION",
        "a caption is one line of at most 100 characters"
    );
    Ok(caption.to_owned())
}

pub fn retakes(count: i32) -> AppResult<i16> {
    ensure!((0..=MAX_RETAKES).contains(&count), invalid, "INVALID_RETAKES", "retakes must be between 0 and 99");
    Ok(count as i16)
}

pub fn location(lat: f64, lng: f64) -> AppResult<(f64, f64)> {
    ensure!(
        tokuly_core::geoindex::valid_coordinates(lat, lng),
        invalid,
        "INVALID_LOCATION",
        "a valid location is required"
    );
    Ok((lat, lng))
}

/// The emoji a library entry can be kept for (all but ⚡).
pub fn library_emoji(emoji: Option<RealmojiEmoji>) -> AppResult<RealmojiEmoji> {
    match emoji {
        Some(emoji)
            if emoji != RealmojiEmoji::REALMOJI_EMOJI_UNSPECIFIED && emoji != RealmojiEmoji::REALMOJI_EMOJI_INSTANT =>
        {
            Ok(emoji)
        }
        _ => bail!(invalid, "INVALID_REACTION", "choose one of the RealMoji faces"),
    }
}

/// What a locked viewer gets of a picture: its shape and a blur.
pub fn placeholder(image: &Image) -> Image {
    Image {
        width: image.width,
        height: image.height,
        thumbhash: image.thumbhash.clone(),
        dominant_color: image.dominant_color.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use rand::{SeedableRng as _, rngs::StdRng};

    use super::*;

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;
    const BERLIN: Tz = chrono_tz::Europe::Berlin;

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc)
    }

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn the_prompt_falls_inside_the_local_window() {
        let mut rng = StdRng::seed_from_u64(7);
        let day = date("2026-11-28");
        for _ in 0..500 {
            let prompt = draw_prompt(TOKYO, day, 9, 22, DateTime::<Utc>::MIN_UTC, &mut rng).unwrap();
            assert!(prompt >= at("2026-11-28T09:00:00+09:00"), "{prompt}");
            assert!(prompt < at("2026-11-28T22:00:00+09:00"), "{prompt}");
        }
    }

    #[test]
    fn the_prompt_is_spread_over_the_window() {
        let mut rng = StdRng::seed_from_u64(1);
        let day = date("2026-11-28");
        let hours: std::collections::HashSet<u32> = (0..300)
            .map(|_| {
                let prompt = draw_prompt(TOKYO, day, 9, 22, DateTime::<Utc>::MIN_UTC, &mut rng).unwrap();
                chrono::Timelike::hour(&prompt.with_timezone(&TOKYO))
            })
            .collect();
        assert_eq!(hours.len(), 13, "every hour of the window comes up: {hours:?}");
    }

    #[test]
    fn a_region_seen_late_gets_what_is_left_of_its_window_or_nothing() {
        let mut rng = StdRng::seed_from_u64(3);
        let day = date("2026-11-28");
        let noon = at("2026-11-28T21:30:00+09:00");
        for _ in 0..100 {
            let prompt = draw_prompt(TOKYO, day, 9, 22, noon, &mut rng).unwrap();
            assert!(prompt >= noon && prompt < at("2026-11-28T22:00:00+09:00"));
        }
        assert_eq!(draw_prompt(TOKYO, day, 9, 22, at("2026-11-28T22:00:00+09:00"), &mut rng), None);
    }

    #[test]
    fn daylight_saving_days_keep_their_local_hours() {
        // Berlin springs forward on 2026-03-29 at 02:00 and falls back on 2026-10-25.
        assert_eq!(local_hour(BERLIN, date("2026-03-29"), 9), at("2026-03-29T09:00:00+02:00"));
        assert_eq!(local_hour(BERLIN, date("2026-10-25"), 9), at("2026-10-25T09:00:00+01:00"));
        // 02:00 does not exist that spring morning: 03:00 does.
        assert_eq!(local_hour(BERLIN, date("2026-03-29"), 2), at("2026-03-29T03:00:00+02:00"));
        // Hour 24 is the next midnight.
        assert_eq!(local_hour(TOKYO, date("2026-11-28"), 24), at("2026-11-29T00:00:00+09:00"));
    }

    #[test]
    fn local_dates_follow_the_region() {
        let now = at("2026-11-28T16:00:00Z");
        assert_eq!(local_date(TOKYO, now), date("2026-11-29"));
        assert_eq!(local_date(BERLIN, now), date("2026-11-28"));
    }

    #[test]
    fn lateness_counts_from_the_end_of_the_window() {
        let announced = at("2026-11-28T12:00:00Z");
        let until = on_time_until(announced, 120);
        assert_eq!(until, at("2026-11-28T12:02:00Z"));
        assert_eq!(late_seconds(until, at("2026-11-28T12:01:00Z"), 60), 0);
        // The upload grace keeps a moment taken in the window on time.
        assert_eq!(late_seconds(until, at("2026-11-28T12:03:00Z"), 60), 0);
        assert_eq!(late_seconds(until, at("2026-11-28T12:03:01Z"), 60), 61);
        assert_eq!(late_seconds(until, at("2026-11-28T15:02:00Z"), 60), 3 * 3600);
        assert_eq!(late_seconds(until, at("2026-11-28T12:02:00.5Z"), 0), 1);
    }

    #[test]
    fn captions_are_one_short_line() {
        assert_eq!(caption("  放課後🍦  ").unwrap(), "放課後🍦");
        assert_eq!(caption(&"あ".repeat(100)).unwrap().chars().count(), 100);
        assert_eq!(caption(&"あ".repeat(101)).unwrap_err().reason, "INVALID_CAPTION");
        assert_eq!(caption("一行目\n二行目").unwrap_err().reason, "INVALID_CAPTION");
    }

    #[test]
    fn retakes_and_locations_are_checked() {
        assert_eq!(retakes(0).unwrap(), 0);
        assert_eq!(retakes(99).unwrap(), 99);
        assert_eq!(retakes(-1).unwrap_err().reason, "INVALID_RETAKES");
        assert_eq!(retakes(100).unwrap_err().reason, "INVALID_RETAKES");
        assert!(location(35.68, 139.76).is_ok());
        assert_eq!(location(f64::NAN, 0.0).unwrap_err().reason, "INVALID_LOCATION");
        assert_eq!(location(91.0, 0.0).unwrap_err().reason, "INVALID_LOCATION");
    }

    #[test]
    fn the_instant_face_is_never_kept() {
        assert!(library_emoji(Some(RealmojiEmoji::REALMOJI_EMOJI_SMILE)).is_ok());
        assert!(library_emoji(Some(RealmojiEmoji::REALMOJI_EMOJI_INSTANT)).is_err());
        assert!(library_emoji(None).is_err());
    }

    #[test]
    fn a_locked_picture_keeps_only_its_shape_and_blur() {
        let image = Image {
            asset_id: "a".into(),
            url: "https://media/a.webp".into(),
            width: 1080,
            height: 1440,
            thumbhash: "hash".into(),
            dominant_color: "#336699".into(),
            ..Default::default()
        };
        let shown = placeholder(&image);
        assert_eq!((shown.width, shown.height, shown.thumbhash.as_str()), (1080, 1440, "hash"));
        assert!(shown.url.is_empty() && shown.asset_id.is_empty() && shown.variants.is_empty());
    }
}
