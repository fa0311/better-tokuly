//! Validation of what clients write: texts, emoji, days, time zones,
//! currencies, coordinates and lists of people.
use std::collections::HashSet;

use chrono::NaiveDate;
use chrono_tz::Tz;
use tokuly_core::prelude::*;
use tokuly_proto::common::LatLng;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::dates;

/// Characters as people count them.
pub(crate) fn length(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Trims a text and drops control characters. Line breaks survive only in
/// multi-line texts; in single-line ones they become spaces.
fn clean(input: &str, multiline: bool) -> String {
    let kept: String = input
        .chars()
        .filter_map(|c| match c {
            '\n' if multiline => Some('\n'),
            '\n' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    kept.trim().to_owned()
}

/// A single-line text that must not be empty (a title, an item).
pub(crate) fn required_line(
    input: &str,
    max: usize,
    missing: &'static str,
    too_long: &'static str,
) -> AppResult<String> {
    let text = clean(input, false);
    ensure!(!text.is_empty(), invalid, missing, "this text is required");
    ensure!(length(&text) <= max, invalid, too_long, "at most {max} characters");
    Ok(text)
}

/// An optional single-line text (a note, a comment); empty stays empty.
pub(crate) fn line(input: &str, max: usize, too_long: &'static str) -> AppResult<String> {
    let text = clean(input, false);
    ensure!(length(&text) <= max, invalid, too_long, "at most {max} characters");
    Ok(text)
}

/// An optional text that may span lines (a description).
pub(crate) fn paragraph(input: &str, max: usize, too_long: &'static str) -> AppResult<String> {
    let text = clean(input, true);
    ensure!(length(&text) <= max, invalid, too_long, "at most {max} characters");
    Ok(text)
}

/// Nothing, or exactly one emoji.
pub(crate) fn emoji(input: &str) -> AppResult<String> {
    let text = input.trim();
    let one_symbol = length(text) <= 1
        && text.len() <= 64
        && !text.chars().any(|c| c.is_alphanumeric() || c.is_whitespace() || c.is_control() || c.is_ascii());
    ensure!(text.is_empty() || one_symbol, invalid, "EMOJI_INVALID", "emoji must be a single emoji");
    Ok(text.to_owned())
}

/// A day written as `YYYY-MM-DD`.
pub(crate) fn date(input: &str, reason: &'static str) -> AppResult<NaiveDate> {
    dates::parse_date(input).ok_or_else(|| AppError::invalid(reason, "dates are written YYYY-MM-DD"))
}

/// An IANA time zone name; empty means `default`.
pub(crate) fn zone(input: &str, default: Tz) -> AppResult<Tz> {
    if input.is_empty() {
        return Ok(default);
    }
    input.parse().map_err(|_| AppError::invalid("TIME_ZONE_INVALID", format!("unknown time zone: {input:?}")))
}

/// An ISO 4217 currency code; empty means `default`.
pub(crate) fn currency(input: &str, default: &str) -> AppResult<String> {
    let code = if input.is_empty() { default } else { input };
    ensure!(
        code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()),
        invalid,
        "CURRENCY_INVALID",
        "currency must be a three-letter ISO 4217 code such as JPY"
    );
    Ok(code.to_owned())
}

/// A coordinate that exists on Earth, or none when the field is unset.
pub(crate) fn coordinates(value: Option<&LatLng>, reason: &'static str) -> AppResult<Option<(f64, f64)>> {
    let Some(point) = value else { return Ok(None) };
    let on_earth = point.lat.is_finite()
        && point.lng.is_finite()
        && (-90.0..=90.0).contains(&point.lat)
        && (-180.0..=180.0).contains(&point.lng);
    ensure!(on_earth, invalid, reason, "coordinates are out of range");
    Ok(Some((point.lat, point.lng)))
}

/// User IDs from a request, each once and in the order given, without `except`.
pub(crate) fn people<S: AsRef<str>>(values: &[S], except: Uuid) -> AppResult<Vec<Uuid>> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let id = parse_id(value.as_ref())?;
        if id != except && seen.insert(id) {
            out.push(id);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_are_trimmed_and_measured_in_characters() {
        assert_eq!(required_line("  カラオケ  ", 5, "X_REQUIRED", "X_TOO_LONG").unwrap(), "カラオケ");
        assert_eq!(required_line("   ", 5, "X_REQUIRED", "X_TOO_LONG").unwrap_err().reason, "X_REQUIRED");
        assert_eq!(required_line("カラオケ行こう", 5, "X_REQUIRED", "X_TOO_LONG").unwrap_err().reason, "X_TOO_LONG");
        // A family emoji is one character to people, however many code points it has.
        assert_eq!(line("👨‍👩‍👧‍👦👍", 2, "X_TOO_LONG").unwrap(), "👨‍👩‍👧‍👦👍");
        assert_eq!(line("a\nb\u{0007}c", 10, "X_TOO_LONG").unwrap(), "a bc");
        assert_eq!(paragraph(" a\nb ", 10, "X_TOO_LONG").unwrap(), "a\nb");
    }

    #[test]
    fn an_emoji_is_one_symbol_or_nothing() {
        for good in ["", "🎤", "👨‍👩‍👧‍👦", "🇯🇵", "❤️"] {
            assert_eq!(emoji(good).unwrap(), good);
        }
        assert_eq!(emoji(" 🎤 ").unwrap(), "🎤");
        for bad in ["a", "🎤🎤", "ab", "1", "🎤 a", "カ"] {
            assert_eq!(emoji(bad).unwrap_err().reason, "EMOJI_INVALID", "{bad}");
        }
    }

    #[test]
    fn zones_and_currencies_have_defaults() {
        assert_eq!(zone("", chrono_tz::Asia::Tokyo).unwrap(), chrono_tz::Asia::Tokyo);
        assert_eq!(zone("Europe/Paris", chrono_tz::Asia::Tokyo).unwrap(), chrono_tz::Europe::Paris);
        assert_eq!(zone("Nowhere", chrono_tz::Asia::Tokyo).unwrap_err().reason, "TIME_ZONE_INVALID");
        assert_eq!(currency("", "JPY").unwrap(), "JPY");
        assert_eq!(currency("USD", "JPY").unwrap(), "USD");
        for bad in ["usd", "US", "YENS", "¥¥¥"] {
            assert_eq!(currency(bad, "JPY").unwrap_err().reason, "CURRENCY_INVALID");
        }
    }

    #[test]
    fn coordinates_must_be_on_earth() {
        let point = |lat, lng| LatLng { lat, lng, ..Default::default() };
        assert_eq!(coordinates(None, "AREA_INVALID").unwrap(), None);
        assert_eq!(coordinates(Some(&point(35.6, 139.7)), "AREA_INVALID").unwrap(), Some((35.6, 139.7)));
        for bad in [point(91.0, 0.0), point(0.0, 181.0), point(f64::NAN, 0.0)] {
            assert_eq!(coordinates(Some(&bad), "AREA_INVALID").unwrap_err().reason, "AREA_INVALID");
        }
    }

    #[test]
    fn people_are_listed_once_without_the_caller() {
        let (me, a, b) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let ids = [a.to_string(), me.to_string(), b.to_string(), a.to_string()];
        assert_eq!(people(&ids, me).unwrap(), vec![a, b]);
        assert_eq!(people(&["nope"], me).unwrap_err().reason, "INVALID_ID");
    }
}
