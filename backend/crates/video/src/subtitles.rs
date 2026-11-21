//! Subtitle tracks: parsing WebVTT and SubRip (SRT), and storing tracks.
//!
//! Whatever is uploaded is parsed into cues and written back out as a clean
//! WebVTT file: timings are validated and normalized, cues sorted, comments,
//! style and region blocks dropped. That file is stored in the media bucket
//! under a content-addressed key and served publicly like the video's
//! streams (`<track src>` needs a plain URL).
use tokuly_core::{keys::sha256_hex, prelude::*, storage::Bucket};
use tokuly_media::assets;
use tokuly_proto::{
    media::AssetPurpose,
    video::{SubtitleFormat, SubtitleTrack},
};

use crate::{validate, videos::VideoRow};

/// Largest subtitle file accepted, in bytes.
pub const MAX_BYTES: usize = 1024 * 1024;
/// Most cues one track may have.
pub const MAX_CUES: usize = 50_000;
/// Most tracks one video may have.
pub const MAX_TRACKS: i64 = 50;

const FORMAT_VTT: i32 = SubtitleFormat::SUBTITLE_FORMAT_VTT as i32;
const FORMAT_SRT: i32 = SubtitleFormat::SUBTITLE_FORMAT_SRT as i32;

// ───────────────────────────── parsing ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    /// WebVTT cue settings (`line:0 align:start`), empty for SRT.
    pub settings: String,
    pub text: String,
}

/// A parsed subtitle file, ready to store.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// What the file was uploaded as (`SubtitleFormat` number).
    pub format: i32,
    pub cues: Vec<Cue>,
}

fn invalid(message: &str) -> AppError {
    AppError::invalid("SUBTITLE_INVALID", message.to_owned())
}

/// Parses `[hh:]mm:ss.ttt` (WebVTT) or `hh:mm:ss,ttt` (SRT) into milliseconds.
fn parse_timestamp(input: &str) -> Option<u64> {
    let input = input.trim();
    let (clock, fraction) = match input.split_once(['.', ',']) {
        Some((clock, fraction)) => (clock, fraction),
        None => (input, "0"),
    };
    if fraction.is_empty() || fraction.len() > 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let millis = fraction.parse::<u64>().ok()? * 10u64.pow(3 - fraction.len() as u32);
    let parts: Vec<&str> = clock.split(':').collect();
    let number = |part: &str, max_len: usize| -> Option<u64> {
        (!part.is_empty() && part.len() <= max_len && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    };
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, number(minutes, 2)?, number(seconds, 2)?),
        [hours, minutes, seconds] => (number(hours, 4)?, number(minutes, 2)?, number(seconds, 2)?),
        _ => return None,
    };
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    Some(((hours * 60 + minutes) * 60 + seconds) * 1000 + millis)
}

fn format_timestamp(ms: u64) -> String {
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Parses a cue timing line; returns `(start, end, rest of the line)`.
fn parse_timing(line: &str) -> Option<(u64, u64, &str)> {
    let (start, rest) = line.split_once("-->")?;
    let rest = rest.trim_start();
    let (end, settings) = match rest.split_once([' ', '\t']) {
        Some((end, settings)) => (end, settings.trim()),
        None => (rest, ""),
    };
    let (start, end) = (parse_timestamp(start)?, parse_timestamp(end)?);
    (end > start).then_some((start, end, settings))
}

/// Keeps cue settings only when they are plain `name:value` pairs.
fn clean_settings(settings: &str) -> String {
    let plain = settings.split_whitespace().all(|pair| {
        pair.split_once(':').is_some_and(|(name, value)| {
            !name.is_empty()
                && name.bytes().all(|b| b.is_ascii_alphabetic())
                && !value.is_empty()
                && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'%' | b',' | b'.' | b'-'))
        })
    });
    if plain { settings.split_whitespace().collect::<Vec<_>>().join(" ") } else { String::new() }
}

/// Cue text as it goes into the WebVTT file: no arrows (they would start a
/// new cue) and no blank lines (they would end it).
fn clean_text(lines: &[&str]) -> String {
    lines
        .iter()
        .map(|line| line.trim_end().replace("-->", "→"))
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Removes what SubRip files carry that WebVTT does not know: `{\an8}`
/// style overrides and `<font>` tags. `<b>`, `<i>` and `<u>` mean the same
/// in both formats and stay.
fn clean_srt_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find(['{', '<']) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let close = if tail.starts_with('{') { '}' } else { '>' };
        match tail.find(close) {
            Some(end) => {
                let tag = &tail[..=end];
                let inner = tag[1..tag.len() - 1].trim().to_ascii_lowercase();
                let keep = close == '>' && matches!(inner.as_str(), "b" | "/b" | "i" | "/i" | "u" | "/u");
                let drop = (close == '}' && inner.starts_with('\\'))
                    || (close == '>' && (inner.starts_with("font") || inner == "/font"));
                if keep || !drop {
                    out.push_str(tag);
                }
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(tail);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn blocks(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.split("\n\n")
        .map(|block| block.lines().filter(|line| !line.trim().is_empty()).collect::<Vec<_>>())
        .filter(|lines| !lines.is_empty())
}

fn parse_vtt(text: &str) -> AppResult<Vec<Cue>> {
    let mut cues = Vec::new();
    // The first block is the header (`WEBVTT`, optionally with metadata lines).
    for lines in blocks(text).skip(1) {
        let first = lines[0].trim_start();
        if first.starts_with("NOTE") || first.starts_with("STYLE") || first.starts_with("REGION") {
            continue;
        }
        // An optional identifier line precedes the timing line.
        let timing_at = if lines[0].contains("-->") { 0 } else { 1 };
        let timing = lines.get(timing_at).and_then(|line| parse_timing(line));
        let Some((start_ms, end_ms, settings)) = timing else {
            return Err(invalid("a cue has no valid timing line (hh:mm:ss.ttt --> hh:mm:ss.ttt)"));
        };
        let text = clean_text(&lines[timing_at + 1..]);
        if !text.is_empty() {
            cues.push(Cue { start_ms, end_ms, settings: clean_settings(settings), text });
        }
    }
    Ok(cues)
}

fn parse_srt(text: &str) -> AppResult<Vec<Cue>> {
    let mut cues = Vec::new();
    for lines in blocks(text) {
        let numbered = lines[0].trim().bytes().all(|b| b.is_ascii_digit());
        let timing_at = usize::from(numbered && !lines[0].contains("-->"));
        let timing = lines.get(timing_at).and_then(|line| parse_timing(line));
        let Some((start_ms, end_ms, _)) = timing else {
            return Err(invalid("not a WebVTT or SRT file, or a cue has no valid timing line"));
        };
        let cleaned: Vec<String> = lines[timing_at + 1..].iter().map(|line| clean_srt_line(line)).collect();
        let text = clean_text(&cleaned.iter().map(String::as_str).collect::<Vec<_>>());
        if !text.is_empty() {
            cues.push(Cue { start_ms, end_ms, settings: String::new(), text });
        }
    }
    Ok(cues)
}

/// Parses a WebVTT or SRT file. Fails with `SUBTITLE_TOO_LARGE` or
/// `SUBTITLE_INVALID` (not text, neither format, malformed timings, no cues).
pub fn parse(bytes: &[u8]) -> AppResult<Parsed> {
    ensure!(bytes.len() <= MAX_BYTES, invalid, "SUBTITLE_TOO_LARGE", "subtitle files are at most 1 MiB");
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("subtitle files must be UTF-8 text"))?;
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n").replace('\0', "");
    // A line of only spaces separates cues like an empty one.
    let text = text.lines().map(|line| if line.trim().is_empty() { "" } else { line }).collect::<Vec<_>>().join("\n");
    let body = text.trim_start();
    let is_vtt = body.strip_prefix("WEBVTT").is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t', '\n']));
    let (format, mut cues) = if is_vtt { (FORMAT_VTT, parse_vtt(body)?) } else { (FORMAT_SRT, parse_srt(body)?) };
    if cues.is_empty() {
        return Err(invalid("the file has no cues"));
    }
    if cues.len() > MAX_CUES {
        return Err(invalid("the file has too many cues"));
    }
    cues.sort_by_key(|cue| (cue.start_ms, cue.end_ms));
    Ok(Parsed { format, cues })
}

impl Parsed {
    /// The track as a WebVTT file.
    pub fn to_vtt(&self) -> String {
        let mut out = String::from("WEBVTT\n");
        for cue in &self.cues {
            out.push('\n');
            out.push_str(&format_timestamp(cue.start_ms));
            out.push_str(" --> ");
            out.push_str(&format_timestamp(cue.end_ms));
            if !cue.settings.is_empty() {
                out.push(' ');
                out.push_str(&cue.settings);
            }
            out.push('\n');
            out.push_str(&cue.text);
            out.push('\n');
        }
        out
    }
}

// ───────────────────────────── storage ─────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SubtitleRow {
    pub id: Uuid,
    pub video_id: Uuid,
    pub language: String,
    pub label: String,
    pub object_key: String,
    pub is_default: bool,
    pub original_format: i32,
    pub cue_count: i32,
    pub created_at: Time,
}

const COLUMNS: &str =
    "s.id, s.video_id, s.language, s.label, s.object_key, s.is_default, s.original_format, s.cue_count, s.created_at";

impl SubtitleRow {
    pub fn to_proto(&self, core: &Core) -> SubtitleTrack {
        SubtitleTrack {
            id: self.id.to_string(),
            language: self.language.clone(),
            label: self.label.clone(),
            url: core.storage.media_url(&self.object_key),
            is_default: self.is_default,
            original_format: self.original_format.into(),
            cue_count: self.cue_count,
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

/// Key prefix of every subtitle file of a video.
pub fn object_prefix(video_id: Uuid) -> String {
    format!("subtitles/{video_id}/")
}

fn object_key(video_id: Uuid, subtitle_id: Uuid, vtt: &str) -> String {
    format!("{}{subtitle_id}-{}.vtt", object_prefix(video_id), &sha256_hex(vtt.as_bytes())[..16])
}

async fn store(core: &Core, key: &str, vtt: String) -> AppResult<()> {
    core.storage
        .put_with_cache(
            Bucket::Media,
            key,
            vtt.into_bytes(),
            "text/vtt; charset=utf-8",
            Some("public, max-age=31536000, immutable"),
        )
        .await
}

/// Where the content of a track comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// An upload with `ASSET_PURPOSE_SUBTITLE` owned by the caller.
    Asset(Uuid),
    /// The file's text.
    Content(String),
}

/// Reads and parses a source. An uploaded asset is only a way to get the
/// bytes here: the track keeps its own copy, so the upload is deleted.
async fn load(core: &Core, owner_id: Uuid, source: Source) -> AppResult<Parsed> {
    match source {
        Source::Content(content) => parse(content.as_bytes()),
        Source::Asset(asset_id) => {
            let asset =
                assets::require_ready(core, asset_id, owner_id, &[AssetPurpose::ASSET_PURPOSE_SUBTITLE]).await?;
            ensure!(
                asset.size_bytes <= MAX_BYTES as i64,
                invalid,
                "SUBTITLE_TOO_LARGE",
                "subtitle files are at most 1 MiB"
            );
            let bytes = assets::read_original(core, &asset, MAX_BYTES as i64).await?;
            let parsed = parse(&bytes)?;
            if let Err(error) = assets::delete(core, asset_id, Some(owner_id)).await {
                tracing::debug!(%asset_id, %error, "subtitle upload not deleted; the media janitor will collect it");
            }
            Ok(parsed)
        }
    }
}

/// Tracks of a video: the default one first, then by language.
pub async fn list(core: &Core, video_id: Uuid) -> AppResult<Vec<SubtitleTrack>> {
    let rows: Vec<SubtitleRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM video_subtitles s WHERE s.video_id = $1 ORDER BY s.is_default DESC, s.language, s.id"
    )))
    .bind(video_id)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.iter().map(|row| row.to_proto(core)).collect())
}

/// A track that belongs to a video of `owner_id`.
pub async fn find_owned(core: &Core, owner_id: Uuid, subtitle_id: Uuid) -> AppResult<SubtitleRow> {
    let row: Option<SubtitleRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM video_subtitles s JOIN videos v ON v.id = s.video_id WHERE s.id = $1 AND v.owner_id = $2"
    )))
    .bind(subtitle_id)
    .bind(owner_id)
    .fetch_optional(&core.db)
    .await?;
    row.or_not_found("SUBTITLE_NOT_FOUND", "subtitle track not found")
}

async fn find(core: &Core, subtitle_id: Uuid) -> AppResult<SubtitleRow> {
    let row: Option<SubtitleRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM video_subtitles s WHERE s.id = $1")))
            .bind(subtitle_id)
            .fetch_optional(&core.db)
            .await?;
    row.or_not_found("SUBTITLE_NOT_FOUND", "subtitle track not found")
}

fn language_exists(error: sqlx::Error) -> AppError {
    let error = AppError::from(error);
    if error.is_already_exists() {
        AppError::already_exists("SUBTITLE_LANGUAGE_EXISTS", "this video already has a track in that language")
    } else {
        error
    }
}

/// Adds a track to `video` (which the caller owns).
pub async fn add(
    core: &Core,
    video: &VideoRow,
    source: Source,
    language: &str,
    label: &str,
    is_default: bool,
) -> AppResult<SubtitleTrack> {
    let language = validate::subtitle_language(language)?;
    let label = validate::subtitle_label(label, &language)?;
    let (count, taken): (i64, bool) = sqlx::query_as(
        "SELECT count(*), COALESCE(bool_or(language = $2), false) FROM video_subtitles WHERE video_id = $1",
    )
    .bind(video.id)
    .bind(&language)
    .fetch_one(&core.db)
    .await?;
    ensure!(!taken, already_exists, "SUBTITLE_LANGUAGE_EXISTS", "this video already has a track in that language");
    ensure!(
        count < MAX_TRACKS,
        precondition,
        "SUBTITLE_LIMIT_REACHED",
        "a video can have at most {MAX_TRACKS} subtitle tracks"
    );
    let parsed = load(core, video.owner_id, source).await?;

    let subtitle_id = new_id();
    let vtt = parsed.to_vtt();
    let key = object_key(video.id, subtitle_id, &vtt);
    store(core, &key, vtt).await?;
    let mut tx = core.db.begin().await?;
    if is_default {
        sqlx::query("UPDATE video_subtitles SET is_default = false WHERE video_id = $1 AND is_default")
            .bind(video.id)
            .execute(&mut *tx)
            .await?;
    }
    let inserted = sqlx::query(
        "INSERT INTO video_subtitles (id, video_id, language, label, object_key, is_default, original_format, cue_count)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(subtitle_id)
    .bind(video.id)
    .bind(&language)
    .bind(&label)
    .bind(&key)
    .bind(is_default)
    .bind(parsed.format)
    .bind(parsed.cues.len() as i32)
    .execute(&mut *tx)
    .await
    .map_err(language_exists);
    if let Err(error) = inserted {
        drop(tx);
        let _ = core.storage.delete(Bucket::Media, &key).await;
        return Err(error);
    }
    tx.commit().await?;
    Ok(find(core, subtitle_id).await?.to_proto(core))
}

/// Changes of a track; `None` leaves a field as it is.
#[derive(Debug, Default)]
pub struct SubtitleEdit {
    pub language: Option<String>,
    pub label: Option<String>,
    pub is_default: Option<bool>,
    pub source: Option<Source>,
}

pub async fn update(core: &Core, owner_id: Uuid, track: &SubtitleRow, edit: SubtitleEdit) -> AppResult<SubtitleTrack> {
    let language = edit.language.as_deref().map(validate::subtitle_language).transpose()?;
    let label = match &edit.label {
        Some(label) => Some(validate::subtitle_label(label, language.as_deref().unwrap_or(&track.language))?),
        None => None,
    };
    let content = match edit.source {
        Some(source) => {
            let parsed = load(core, owner_id, source).await?;
            let vtt = parsed.to_vtt();
            let key = object_key(track.video_id, track.id, &vtt);
            if key != track.object_key {
                store(core, &key, vtt).await?;
            }
            Some((key, parsed))
        }
        None => None,
    };

    let mut tx = core.db.begin().await?;
    if edit.is_default == Some(true) {
        sqlx::query("UPDATE video_subtitles SET is_default = false WHERE video_id = $1 AND is_default AND id <> $2")
            .bind(track.video_id)
            .bind(track.id)
            .execute(&mut *tx)
            .await?;
    }
    let updated = sqlx::query(
        "UPDATE video_subtitles SET language = COALESCE($2, language), label = COALESCE($3, label),
                is_default = COALESCE($4, is_default), object_key = COALESCE($5, object_key),
                original_format = COALESCE($6, original_format), cue_count = COALESCE($7, cue_count)
         WHERE id = $1",
    )
    .bind(track.id)
    .bind(&language)
    .bind(&label)
    .bind(edit.is_default)
    .bind(content.as_ref().map(|(key, _)| key))
    .bind(content.as_ref().map(|(_, parsed)| parsed.format))
    .bind(content.as_ref().map(|(_, parsed)| parsed.cues.len() as i32))
    .execute(&mut *tx)
    .await
    .map_err(language_exists);
    if let Err(error) = updated {
        drop(tx);
        if let Some((key, _)) = content.as_ref().filter(|(key, _)| *key != track.object_key) {
            let _ = core.storage.delete(Bucket::Media, key).await;
        }
        return Err(error);
    }
    tx.commit().await?;
    // The previous file is no longer referenced.
    if content.as_ref().is_some_and(|(key, _)| *key != track.object_key) {
        if let Err(error) = core.storage.delete(Bucket::Media, &track.object_key).await {
            tracing::warn!(key = %track.object_key, %error, "could not delete a replaced subtitle file");
        }
    }
    Ok(find(core, track.id).await?.to_proto(core))
}

pub async fn delete(core: &Core, track: &SubtitleRow) -> AppResult<()> {
    sqlx::query("DELETE FROM video_subtitles WHERE id = $1").bind(track.id).execute(&core.db).await?;
    if let Err(error) = core.storage.delete(Bucket::Media, &track.object_key).await {
        tracing::warn!(key = %track.object_key, %error, "could not delete a subtitle file");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_parse_in_both_dialects() {
        assert_eq!(parse_timestamp("00:00:01.000"), Some(1000));
        assert_eq!(parse_timestamp("01:02.5"), Some(62_500));
        assert_eq!(parse_timestamp("1:00:00,250"), Some(3_600_250));
        assert_eq!(parse_timestamp("100:59:59.999"), Some(363_599_999));
        for bad in ["", "1", "00:60:00.000", "00:00:61.000", "a:b:c", "00:00:01.0000", "00:00:01.", "-1:00.000"] {
            assert_eq!(parse_timestamp(bad), None, "{bad}");
        }
        assert_eq!(format_timestamp(3_723_004), "01:02:03.004");
    }

    #[test]
    fn webvtt_is_normalized() {
        let source = "\u{feff}WEBVTT - made by hand\r\n\r\nNOTE a comment\r\nover two lines\r\n\r\nSTYLE\r\n::cue { color: red }\r\n\r\n\
                      intro\r\n00:05.000 --> 00:07.500 line:0 align:start\r\n<v とく>こんにちは!\r\n二行目\r\n\r\n\
                      00:00:01.000 --> 00:00:02.000 bad{settings}\r\nfirst\r\n";
        let parsed = parse(source.as_bytes()).unwrap();
        assert_eq!(parsed.format, FORMAT_VTT);
        assert_eq!(
            parsed.to_vtt(),
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nfirst\n\n00:00:05.000 --> 00:00:07.500 line:0 align:start\n<v とく>こんにちは!\n二行目\n"
        );
    }

    #[test]
    fn srt_is_converted() {
        let source = "1\n00:00:01,000 --> 00:00:03,200\n{\\an8}<font color=\"#fff\">Hello</font> <i>there</i>\n\n\n2\n00:00:04,000 --> 00:00:05,000 X1:0 X2:10\nan arrow --> here\n";
        let parsed = parse(source.as_bytes()).unwrap();
        assert_eq!(parsed.format, FORMAT_SRT);
        assert_eq!(
            parsed.to_vtt(),
            "WEBVTT\n\n00:00:01.000 --> 00:00:03.200\nHello <i>there</i>\n\n00:00:04.000 --> 00:00:05.000\nan arrow → here\n"
        );
        // What comes out is itself valid WebVTT with the same cues.
        let again = parse(parsed.to_vtt().as_bytes()).unwrap();
        assert_eq!((again.format, again.cues.len()), (FORMAT_VTT, 2));
        assert_eq!(again.cues, parsed.cues);
    }

    #[test]
    fn broken_files_are_rejected_with_reasons() {
        let reason = |bytes: &[u8]| parse(bytes).unwrap_err().reason;
        assert_eq!(reason(b"just some text"), "SUBTITLE_INVALID");
        assert_eq!(reason(b"WEBVTT\n\n"), "SUBTITLE_INVALID");
        assert_eq!(reason(b"WEBVTT\n\n00:00:05.000 --> 00:00:01.000\nbackwards\n"), "SUBTITLE_INVALID");
        assert_eq!(reason(b"WEBVTT\n\nid\nnot a timing\ntext\n"), "SUBTITLE_INVALID");
        assert_eq!(reason(b"1\n00:00:01,000 -> 00:00:02,000\nwrong arrow\n"), "SUBTITLE_INVALID");
        assert_eq!(reason(&[0xff, 0xfe, 0x00]), "SUBTITLE_INVALID");
        assert_eq!(reason(&vec![b'a'; MAX_BYTES + 1]), "SUBTITLE_TOO_LARGE");
        // "WEBVTTX" is not the WebVTT signature.
        assert_eq!(reason(b"WEBVTTX\n\n00:00:01.000 --> 00:00:02.000\nx\n"), "SUBTITLE_INVALID");
    }

    #[test]
    fn cues_without_text_are_dropped_and_the_rest_sorted() {
        let parsed = parse(b"WEBVTT\n\n00:00:09.000 --> 00:00:10.000\nlast\n\n00:00:03.000 --> 00:00:04.000\n\n00:00:01.000 --> 00:00:02.000\nfirst\n").unwrap();
        assert_eq!(parsed.cues.iter().map(|cue| cue.text.as_str()).collect::<Vec<_>>(), vec!["first", "last"]);
    }

    #[test]
    fn object_keys_change_with_the_content() {
        let (video, track) = (Uuid::now_v7(), Uuid::now_v7());
        let a = object_key(video, track, "WEBVTT\n\na");
        assert!(a.starts_with(&object_prefix(video)) && a.ends_with(".vtt"));
        assert_ne!(a, object_key(video, track, "WEBVTT\n\nb"));
    }
}
