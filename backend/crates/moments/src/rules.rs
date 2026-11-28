//! Everything free of I/O: the prompt regions and when their prompts go
//! out, how late a moment is, what a request may contain, and what a
//! locked viewer gets of a picture.
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Offset as _, Utc};
use chrono_tz::Tz;
use rand::{Rng, RngExt as _};
use tokuly_core::prelude::*;
use tokuly_proto::{common::Image, moments::RealmojiEmoji};

/// The longest caption, in characters.
pub const MAX_CAPTION_CHARS: usize = 100;
/// The most retakes an app may report.
pub const MAX_RETAKES: i32 = 99;

/// A prompt region: the time zones whose UTC offset (right now, daylight
/// saving included) is between `west` and `east` minutes, both included.
/// Everyone in it is prompted at the same moment, so a band is narrow
/// enough (4 hours at most) for that moment to fall inside the local window
/// everywhere in it; and there are few of them, so prompts go out six times
/// a day around the world, not every few minutes somewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    /// The region's name, `moment_regions.region`.
    pub name: &'static str,
    pub west: i32,
    pub east: i32,
}

/// The prompt regions, west to east. The seams lie where few people live:
/// the eastern Pacific, the mid-Atlantic, between the Gulf and Iran, the
/// Bay of Bengal and the Tasman Sea.
pub const BANDS: [Band; 6] = [
    Band { name: "utc-11..-7", west: -11 * 60, east: -7 * 60 },
    Band { name: "utc-6..-2", west: -6 * 60, east: -2 * 60 },
    Band { name: "utc-1..+3", west: -60, east: 3 * 60 },
    Band { name: "utc+3:30..+6:30", west: 3 * 60 + 30, east: 6 * 60 + 30 },
    Band { name: "utc+7..+11", west: 7 * 60, east: 11 * 60 },
    Band { name: "utc+12..+14", west: 12 * 60, east: 14 * 60 },
];

/// The widest band, in hours: the local window must be longer.
pub const WIDEST_BAND_HOURS: u32 = 4;

/// The band of a UTC offset (minutes east): the first that reaches that far
/// east. The one offset west of every band (Etc/GMT+12, where nobody lives)
/// joins the first.
pub fn band_of(offset_minutes: i32) -> &'static Band {
    BANDS.iter().find(|band| offset_minutes <= band.east).unwrap_or(&BANDS[BANDS.len() - 1])
}

/// The UTC offset of `zone` at `now`, in minutes east.
pub fn offset_minutes(zone: Tz, now: DateTime<Utc>) -> i32 {
    now.with_timezone(&zone).offset().fix().local_minus_utc() / 60
}

/// The date `now` falls on in `band`: the date at its western edge (the
/// window of a date lies inside that date everywhere in the band).
pub fn band_date(band: &Band, now: DateTime<Utc>) -> NaiveDate {
    (now + Duration::minutes(i64::from(band.west))).date_naive()
}

/// Draws the prompt time of `date` in `band`: uniformly inside the part of
/// the local window `[from_hour, until_hour)` that every offset of the band
/// shares, and not before `not_before` (a band drawn in the middle of its
/// day). `None` when nothing of the window is left.
pub fn draw_prompt<R: Rng + ?Sized>(
    band: &Band,
    date: NaiveDate,
    from_hour: u32,
    until_hour: u32,
    not_before: DateTime<Utc>,
    rng: &mut R,
) -> Option<DateTime<Utc>> {
    let midnight = date.and_time(NaiveTime::MIN).and_utc();
    // `from_hour` at the western edge is the latest start, `until_hour` at
    // the eastern edge the earliest end.
    let start =
        (midnight + Duration::hours(i64::from(from_hour)) - Duration::minutes(i64::from(band.west))).max(not_before);
    let end = midnight + Duration::hours(i64::from(until_hour)) - Duration::minutes(i64::from(band.east));
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

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc)
    }

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn band(name: &str) -> &'static Band {
        BANDS.iter().find(|band| band.name == name).unwrap()
    }

    #[test]
    fn the_bands_are_narrow_and_in_order() {
        for pair in BANDS.windows(2) {
            assert!(pair[0].east < pair[1].west, "{pair:?}");
        }
        for band in BANDS {
            assert!(band.west <= band.east && band.east - band.west <= WIDEST_BAND_HOURS as i32 * 60, "{band:?}");
        }
        let winter = at("2026-01-15T12:00:00Z");
        let zone = |name: &str| band_of(offset_minutes(name.parse().unwrap(), winter)).name;
        assert_eq!(zone("Asia/Tokyo"), "utc+7..+11");
        assert_eq!(zone("Asia/Seoul"), "utc+7..+11");
        assert_eq!(zone("Australia/Sydney"), "utc+7..+11");
        assert_eq!(zone("Asia/Kolkata"), "utc+3:30..+6:30");
        assert_eq!(zone("Europe/London"), "utc-1..+3");
        assert_eq!(zone("Europe/Moscow"), "utc-1..+3");
        assert_eq!(zone("America/New_York"), "utc-6..-2");
        assert_eq!(zone("America/Sao_Paulo"), "utc-6..-2");
        assert_eq!(zone("America/Los_Angeles"), "utc-11..-7");
        assert_eq!(zone("Pacific/Honolulu"), "utc-11..-7");
        assert_eq!(zone("Pacific/Auckland"), "utc+12..+14");
        assert_eq!(zone("Etc/GMT+12"), "utc-11..-7", "west of every band: the first");
    }

    #[test]
    fn every_time_zone_is_prompted_inside_its_local_window() {
        let mut rng = StdRng::seed_from_u64(7);
        // A winter and a summer day of both hemispheres.
        for now in [at("2026-01-15T00:00:00Z"), at("2026-07-15T00:00:00Z")] {
            for zone in chrono_tz::TZ_VARIANTS {
                let offset = offset_minutes(zone, now);
                let band = band_of(offset);
                if offset < band.west {
                    continue; // Etc/GMT+12
                }
                let day = band_date(band, now) + chrono::Days::new(1);
                for _ in 0..20 {
                    let prompt = draw_prompt(band, day, 9, 22, DateTime::<Utc>::MIN_UTC, &mut rng).unwrap();
                    let local = prompt + Duration::minutes(i64::from(offset));
                    assert_eq!(local.date_naive(), day, "{zone} {prompt}");
                    let hour = chrono::Timelike::hour(&local);
                    assert!((9..22).contains(&hour), "{zone}: {prompt} is {local} there");
                }
            }
        }
    }

    #[test]
    fn the_prompt_is_spread_over_the_shared_window() {
        let mut rng = StdRng::seed_from_u64(1);
        let tokyo = Duration::hours(9);
        let hours: std::collections::HashSet<u32> = (0..300)
            .map(|_| {
                let prompt =
                    draw_prompt(band("utc+7..+11"), date("2026-11-28"), 9, 22, DateTime::<Utc>::MIN_UTC, &mut rng)
                        .unwrap();
                chrono::Timelike::hour(&(prompt + tokyo))
            })
            .collect();
        // 9:00 in Bangkok to 22:00 in Sydney's summer: 11:00 to 20:00 in Tokyo.
        assert_eq!(hours, (11..20).collect(), "every hour of the shared window comes up");
    }

    #[test]
    fn a_band_drawn_late_gets_what_is_left_of_its_window_or_nothing() {
        let mut rng = StdRng::seed_from_u64(3);
        let (band, day) = (band("utc+7..+11"), date("2026-11-28"));
        let late = at("2026-11-28T19:30:00+09:00");
        for _ in 0..100 {
            let prompt = draw_prompt(band, day, 9, 22, late, &mut rng).unwrap();
            assert!(prompt >= late && prompt < at("2026-11-28T20:00:00+09:00"));
        }
        assert_eq!(draw_prompt(band, day, 9, 22, at("2026-11-28T20:00:00+09:00"), &mut rng), None);
    }

    #[test]
    fn a_band_s_date_is_the_one_at_its_western_edge() {
        let now = at("2026-11-28T16:00:00Z");
        assert_eq!(band_date(band("utc+7..+11"), now), date("2026-11-28"), "23:00 in Bangkok");
        assert_eq!(band_date(band("utc+12..+14"), now), date("2026-11-29"));
        assert_eq!(band_date(band("utc-11..-7"), now), date("2026-11-28"));
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
