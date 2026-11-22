//! Misskey's emoji pack format, read and written.
//!
//! A pack is a ZIP archive with a `meta.json` and one image file per emoji,
//! all at the top level. It is what Misskey's "export custom emojis" job
//! writes and its "import custom emojis" job reads:
//!
//! ```json
//! {
//!   "metaVersion": 2,
//!   "host": "misskey.example",
//!   "exportedAt": "Sat Nov 21 2026 10:00:00 GMT+0900",
//!   "emojis": [
//!     {
//!       "fileName": "blobcat.png",
//!       "downloaded": true,
//!       "emoji": {
//!         "id": "9abc…", "updatedAt": "2026-01-01T00:00:00.000Z",
//!         "name": "blobcat", "host": null, "category": "blobs",
//!         "originalUrl": "https://…", "publicUrl": "https://…", "uri": null,
//!         "type": "image/png", "aliases": ["cat", "ねこ"], "license": "CC0",
//!         "localOnly": false, "isSensitive": false,
//!         "roleIdsThatCanBeUsedThisEmojiAsReaction": []
//!       }
//!     }
//!   ]
//! }
//! ```
//!
//! `downloaded` is false when the exporting server could not fetch the
//! image; such an entry has no file. Of the `emoji` object an import uses
//! `name`, `category`, `aliases`, `license`, `isSensitive` and `localOnly`;
//! the rest describes the emoji on the server it came from.
//!
//! Reading is forgiving: packs are written by several Misskey versions and
//! forks, so every field may be missing or `null`, and unknown fields are
//! ignored. Only a `meta.json` that is not JSON or has no `emojis` array is
//! refused. Everything here is synchronous file work; callers run it on the
//! blocking pool.
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read as _, Write as _},
    path::Path,
};

use serde::Serialize;
use zip::{CompressionMethod, ZipArchive, ZipWriter, result::ZipError, write::SimpleFileOptions};

const META_FILE: &str = "meta.json";
/// Largest `meta.json` read (a pack of 5000 emoji is about 3 MB).
const MAX_META_BYTES: u64 = 32 * 1024 * 1024;
/// Most files an archive may hold; a guard against archives built to exhaust memory.
const MAX_ARCHIVE_FILES: usize = 50_000;
/// The `metaVersion` this code writes.
const META_VERSION: u32 = 2;

/// Why a pack could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    /// The file is not a ZIP archive this code can read.
    ArchiveInvalid,
    /// There is no `meta.json`.
    MetaMissing,
    /// `meta.json` is not JSON, is too large, or has no `emojis` array.
    MetaInvalid,
    /// `meta.json` lists more entries than an import accepts.
    TooManyEntries,
    /// Reading or writing a local file failed.
    Io(String),
}

impl PackError {
    /// The reason reported on the import (`Import.error`).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::ArchiveInvalid => "ARCHIVE_INVALID",
            Self::MetaMissing => "META_MISSING",
            Self::MetaInvalid => "META_INVALID",
            Self::TooManyEntries => "TOO_MANY_ENTRIES",
            Self::Io(_) => "INTERNAL",
        }
    }
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(cause) => write!(f, "pack i/o failed: {cause}"),
            other => f.write_str(other.reason()),
        }
    }
}

fn io_error(error: impl std::fmt::Display) -> PackError {
    PackError::Io(error.to_string())
}

/// One emoji as a pack describes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackEntry {
    /// `emoji.name`, as written by the exporting server.
    pub source_name: String,
    pub file_name: String,
    pub downloaded: bool,
    pub category: Option<String>,
    pub aliases: Vec<String>,
    pub license: String,
    pub sensitive: bool,
    pub local_only: bool,
}

/// The contents of a pack's `meta.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pack {
    /// The server the pack was exported from; empty when it does not say.
    pub host: String,
    pub entries: Vec<PackEntry>,
}

fn text(value: Option<&serde_json::Value>) -> Option<&str> {
    value.and_then(serde_json::Value::as_str)
}

impl PackEntry {
    fn from_json(record: &serde_json::Value) -> Self {
        let emoji = record.get("emoji");
        let field = |name: &str| emoji.and_then(|emoji| emoji.get(name));
        Self {
            source_name: text(field("name")).unwrap_or_default().to_owned(),
            file_name: text(record.get("fileName")).unwrap_or_default().to_owned(),
            // Packs written before the flag existed have a file for every entry.
            downloaded: record.get("downloaded").and_then(serde_json::Value::as_bool).unwrap_or(true),
            category: text(field("category")).map(str::to_owned).filter(|category| !category.trim().is_empty()),
            aliases: field("aliases")
                .and_then(serde_json::Value::as_array)
                .map(|aliases| aliases.iter().filter_map(|alias| alias.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
            license: text(field("license")).unwrap_or_default().to_owned(),
            sensitive: field("isSensitive").and_then(serde_json::Value::as_bool).unwrap_or(false),
            local_only: field("localOnly").and_then(serde_json::Value::as_bool).unwrap_or(false),
        }
    }
}

/// Reads a `meta.json`. Errors: `MetaInvalid`, `TooManyEntries`.
pub fn parse_meta(bytes: &[u8], max_entries: usize) -> Result<Pack, PackError> {
    let meta: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| PackError::MetaInvalid)?;
    let records = meta.get("emojis").and_then(serde_json::Value::as_array).ok_or(PackError::MetaInvalid)?;
    if records.len() > max_entries {
        return Err(PackError::TooManyEntries);
    }
    Ok(Pack {
        host: text(meta.get("host")).unwrap_or_default().chars().take(253).collect(),
        entries: records.iter().map(PackEntry::from_json).collect(),
    })
}

/// Why one image could not be taken out of a pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractError {
    /// The archive has no such file.
    Missing,
    /// The file is larger than an emoji picture may be.
    TooLarge,
    /// The file is damaged, encrypted, or compressed in a way this code does not read.
    Unreadable,
}

impl ExtractError {
    /// The reason reported on the entry (`ImportEntry.reason`).
    pub fn reason(self) -> &'static str {
        match self {
            Self::Missing => "FILE_MISSING",
            Self::TooLarge => "IMAGE_TOO_LARGE",
            Self::Unreadable => "FILE_UNREADABLE",
        }
    }
}

/// A pack opened for reading.
pub struct Archive {
    zip: ZipArchive<File>,
    /// Where `meta.json` and the images are inside the archive: empty for
    /// the top level, or `folder/` when the pack was zipped as a folder.
    root: String,
}

impl Archive {
    /// Opens a pack and finds its `meta.json`.
    /// Errors: `ArchiveInvalid`, `MetaMissing`, `Io`.
    pub fn open(path: &Path) -> Result<Self, PackError> {
        let file = File::open(path).map_err(io_error)?;
        let zip = ZipArchive::new(file).map_err(|_| PackError::ArchiveInvalid)?;
        if zip.len() > MAX_ARCHIVE_FILES {
            return Err(PackError::ArchiveInvalid);
        }
        // Misskey writes everything at the top level; people who zip a
        // folder by hand end up with one level above it.
        let root = zip
            .file_names()
            .filter_map(Result::ok)
            .filter_map(|name| {
                let folder = name.strip_suffix(META_FILE)?;
                (folder.is_empty() || folder.ends_with('/')).then(|| folder.to_owned())
            })
            .min_by_key(String::len)
            .ok_or(PackError::MetaMissing)?;
        Ok(Self { zip, root })
    }

    fn path_of(&self, file_name: &str) -> String {
        format!("{}{file_name}", self.root)
    }

    /// Reads the pack's `meta.json`. Errors: `MetaMissing`, `MetaInvalid`, `TooManyEntries`.
    pub fn meta(&mut self, max_entries: usize) -> Result<Pack, PackError> {
        let path = self.path_of(META_FILE);
        let entry = self.zip.by_name(&path).map_err(|_| PackError::MetaMissing)?;
        let mut bytes = Vec::new();
        entry.take(MAX_META_BYTES + 1).read_to_end(&mut bytes).map_err(|_| PackError::MetaInvalid)?;
        if bytes.len() as u64 > MAX_META_BYTES {
            return Err(PackError::MetaInvalid);
        }
        parse_meta(&bytes, max_entries)
    }

    /// The files next to `meta.json`, with the size each claims to unpack to.
    pub fn file_sizes(&self) -> HashMap<String, u64> {
        let mut sizes = HashMap::new();
        for index in 0..self.zip.len() {
            let Ok(entry) = self.zip.by_index_data(index) else { continue };
            if !entry.is_file() {
                continue;
            }
            let Ok(name) = entry.name() else { continue };
            if let Some(file_name) = name.strip_prefix(self.root.as_str())
                && !file_name.contains('/')
            {
                sizes.insert(file_name.to_owned(), entry.size());
            }
        }
        sizes
    }

    /// Unpacks one image into `to`, refusing anything over `max_bytes` (the
    /// size is enforced while unpacking, whatever the archive claims).
    /// Returns the bytes written.
    pub fn extract(&mut self, file_name: &str, to: &Path, max_bytes: u64) -> Result<u64, ExtractError> {
        let path = self.path_of(file_name);
        let mut entry = self.zip.by_name(&path).map_err(|error| match error {
            ZipError::FileNotFound => ExtractError::Missing,
            _ => ExtractError::Unreadable,
        })?;
        if entry.size() > max_bytes {
            return Err(ExtractError::TooLarge);
        }
        let mut out = File::create(to).map_err(|_| ExtractError::Unreadable)?;
        let written =
            io::copy(&mut (&mut entry).take(max_bytes + 1), &mut out).map_err(|_| ExtractError::Unreadable)?;
        if written > max_bytes {
            return Err(ExtractError::TooLarge);
        }
        Ok(written)
    }
}

/// An emoji to write into a pack.
#[derive(Debug, Clone)]
pub struct ExportedEmoji {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub aliases: Vec<String>,
    pub license: String,
    pub sensitive: bool,
    pub local_only: bool,
    /// Content type of the image file, e.g. `image/png`.
    pub content_type: String,
    /// Where the emoji's image is served.
    pub url: String,
    /// RFC 3339.
    pub updated_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetaOut<'a> {
    meta_version: u32,
    host: &'a str,
    exported_at: &'a str,
    emojis: &'a [RecordOut],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordOut {
    file_name: String,
    downloaded: bool,
    emoji: EmojiOut,
}

/// The `emoji` object of a record, with every field Misskey writes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EmojiOut {
    id: String,
    updated_at: String,
    name: String,
    /// Always `null`: the emoji is local to the exporting server.
    host: Option<String>,
    category: Option<String>,
    original_url: String,
    public_url: String,
    uri: Option<String>,
    r#type: String,
    aliases: Vec<String>,
    license: Option<String>,
    local_only: bool,
    is_sensitive: bool,
    role_ids_that_can_be_used_this_emoji_as_reaction: Vec<String>,
}

/// The file extension Misskey gives an image of this content type.
pub fn extension_for(content_type: &str) -> &'static str {
    match content_type.split(';').next().unwrap_or_default().trim() {
        "image/png" | "image/apng" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/avif" => "avif",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

/// A pack being written.
pub struct Writer {
    zip: ZipWriter<File>,
    records: Vec<RecordOut>,
}

impl Writer {
    pub fn create(path: &Path) -> Result<Self, PackError> {
        let file = File::create(path).map_err(io_error)?;
        Ok(Self { zip: ZipWriter::new(file), records: Vec::new() })
    }

    /// Adds an emoji. `image` is its image file; `None` writes the entry
    /// without a file (`downloaded: false`), as Misskey does for an image it
    /// could not fetch.
    pub fn add(&mut self, emoji: ExportedEmoji, image: Option<&[u8]>) -> Result<(), PackError> {
        let file_name = format!("{}.{}", emoji.name, extension_for(&emoji.content_type));
        if let Some(image) = image {
            // Images are compressed already; storing them keeps writing cheap.
            let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            self.zip.start_file(file_name.as_str(), options).map_err(io_error)?;
            self.zip.write_all(image).map_err(io_error)?;
        }
        self.records.push(RecordOut {
            file_name,
            downloaded: image.is_some(),
            emoji: EmojiOut {
                id: emoji.id,
                updated_at: emoji.updated_at,
                name: emoji.name,
                host: None,
                category: emoji.category,
                original_url: emoji.url.clone(),
                public_url: emoji.url,
                uri: None,
                r#type: emoji.content_type,
                aliases: emoji.aliases,
                license: Some(emoji.license).filter(|license| !license.is_empty()),
                local_only: emoji.local_only,
                is_sensitive: emoji.sensitive,
                role_ids_that_can_be_used_this_emoji_as_reaction: Vec::new(),
            },
        });
        Ok(())
    }

    /// Writes `meta.json` and closes the archive. Returns how many emoji it lists.
    pub fn finish(mut self, host: &str, exported_at: &str) -> Result<usize, PackError> {
        let meta = MetaOut { meta_version: META_VERSION, host, exported_at, emojis: &self.records };
        let json = serde_json::to_vec(&meta).map_err(io_error)?;
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        self.zip.start_file(META_FILE, options).map_err(io_error)?;
        self.zip.write_all(&json).map_err(io_error)?;
        self.zip.finish().map_err(io_error)?.sync_all().map_err(io_error)?;
        Ok(self.records.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `meta.json` as a Misskey server writes it.
    const MISSKEY_META: &str = r#"{"metaVersion":2,"host":"misskey.example","exportedAt":"Sat Nov 21 2026 10:00:00 GMT+0900 (Japan Standard Time)","emojis":[{"fileName":"blobcat.png","downloaded":true,"emoji":{"id":"9abcdefgh1","updatedAt":"2026-01-01T00:00:00.000Z","name":"blobcat","host":null,"category":"blobs","originalUrl":"https://misskey.example/files/a","publicUrl":"https://misskey.example/files/a","uri":null,"type":"image/png","aliases":["cat","ねこ"],"license":"CC0","localOnly":false,"isSensitive":false,"roleIdsThatCanBeUsedThisEmojiAsReaction":[]}},
{"fileName":"Party_Parrot.gif","downloaded":true,"emoji":{"id":"9abcdefgh2","updatedAt":null,"name":"Party_Parrot","host":null,"category":null,"originalUrl":"https://misskey.example/files/b","publicUrl":"https://misskey.example/files/b","uri":null,"type":"image/gif","aliases":[],"license":null,"localOnly":true,"isSensitive":true,"roleIdsThatCanBeUsedThisEmojiAsReaction":["role1"]}},
{"fileName":"gone.png","downloaded":false,"emoji":{"name":"gone"}}]}"#;

    #[test]
    fn a_misskey_meta_file_is_read() {
        let pack = parse_meta(MISSKEY_META.as_bytes(), 10).unwrap();
        assert_eq!(pack.host, "misskey.example");
        assert_eq!(pack.entries.len(), 3);
        assert_eq!(
            pack.entries[0],
            PackEntry {
                source_name: "blobcat".into(),
                file_name: "blobcat.png".into(),
                downloaded: true,
                category: Some("blobs".into()),
                aliases: vec!["cat".into(), "ねこ".into()],
                license: "CC0".into(),
                sensitive: false,
                local_only: false,
            }
        );
        let parrot = &pack.entries[1];
        assert_eq!(
            (parrot.source_name.as_str(), parrot.category.as_deref(), parrot.license.as_str()),
            ("Party_Parrot", None, "")
        );
        assert!(parrot.sensitive && parrot.local_only && parrot.aliases.is_empty());
        assert!(!pack.entries[2].downloaded);
    }

    #[test]
    fn missing_and_odd_fields_do_not_spoil_a_pack() {
        let meta = r#"{"emojis":[{"emoji":{"name":"a_b","aliases":["ok",5,null],"category":"  ","isSensitive":"yes"}},{"fileName":7},{}]}"#;
        let pack = parse_meta(meta.as_bytes(), 10).unwrap();
        assert_eq!(pack.host, "");
        assert_eq!(pack.entries[0].aliases, ["ok"]);
        assert_eq!(pack.entries[0].category, None);
        assert!(!pack.entries[0].sensitive && pack.entries[0].downloaded);
        assert_eq!(pack.entries[1], PackEntry { downloaded: true, ..Default::default() });
        assert_eq!(pack.entries[2].source_name, "");
    }

    #[test]
    fn a_meta_file_without_an_emoji_list_is_refused() {
        assert_eq!(parse_meta(b"not json", 10), Err(PackError::MetaInvalid));
        assert_eq!(parse_meta(br#"{"host":"x"}"#, 10), Err(PackError::MetaInvalid));
        assert_eq!(parse_meta(br#"{"emojis":{}}"#, 10), Err(PackError::MetaInvalid));
        assert_eq!(parse_meta(MISSKEY_META.as_bytes(), 2), Err(PackError::TooManyEntries));
        assert!(parse_meta(br#"{"emojis":[]}"#, 10).unwrap().entries.is_empty());
    }

    #[test]
    fn extensions_follow_the_content_type() {
        assert_eq!(extension_for("image/png"), "png");
        assert_eq!(extension_for("image/gif"), "gif");
        assert_eq!(extension_for("image/webp"), "webp");
        assert_eq!(extension_for("image/jpeg; charset=binary"), "jpg");
        assert_eq!(extension_for("application/zip"), "bin");
    }

    fn exported(name: &str, content_type: &str) -> ExportedEmoji {
        ExportedEmoji {
            id: "0190a000-0000-7000-8000-000000000001".into(),
            name: name.into(),
            category: Some("どうぶつ".into()),
            aliases: vec!["ねこ".into()],
            license: "CC BY 4.0".into(),
            sensitive: false,
            local_only: true,
            content_type: content_type.into(),
            url: format!("https://media.example/{name}"),
            updated_at: "2026-11-21T01:00:00Z".into(),
        }
    }

    #[test]
    fn a_written_pack_reads_back_and_has_the_fields_misskey_expects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pack.zip");
        let mut writer = Writer::create(&path).unwrap();
        writer.add(exported("blobcat", "image/png"), Some(b"png bytes")).unwrap();
        writer.add(exported("lost", "image/gif"), None).unwrap();
        assert_eq!(writer.finish("tokuly.example", "2026-11-22T00:00:00Z").unwrap(), 2);

        let mut archive = Archive::open(&path).unwrap();
        let sizes = archive.file_sizes();
        assert_eq!(sizes.get("blobcat.png"), Some(&9));
        assert!(sizes.contains_key("meta.json") && !sizes.contains_key("lost.gif"));
        let pack = archive.meta(10).unwrap();
        assert_eq!(pack.host, "tokuly.example");
        assert_eq!(pack.entries.len(), 2);
        assert_eq!((pack.entries[0].file_name.as_str(), pack.entries[0].downloaded), ("blobcat.png", true));
        assert_eq!(pack.entries[0].category.as_deref(), Some("どうぶつ"));
        assert!(pack.entries[0].local_only);
        assert_eq!((pack.entries[1].file_name.as_str(), pack.entries[1].downloaded), ("lost.gif", false));

        let out = dir.path().join("blobcat.png");
        assert_eq!(archive.extract("blobcat.png", &out, 100), Ok(9));
        assert_eq!(std::fs::read(&out).unwrap(), b"png bytes");
        assert_eq!(archive.extract("blobcat.png", &out, 5), Err(ExtractError::TooLarge));
        assert_eq!(archive.extract("lost.gif", &out, 100), Err(ExtractError::Missing));

        // The raw JSON carries every key of Misskey's emoji object.
        let mut raw = String::new();
        ZipArchive::new(File::open(&path).unwrap())
            .unwrap()
            .by_name("meta.json")
            .unwrap()
            .read_to_string(&mut raw)
            .unwrap();
        let meta: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(meta["metaVersion"], 2);
        let emoji = &meta["emojis"][0]["emoji"];
        for key in [
            "id",
            "updatedAt",
            "name",
            "host",
            "category",
            "originalUrl",
            "publicUrl",
            "uri",
            "type",
            "aliases",
            "license",
            "localOnly",
            "isSensitive",
            "roleIdsThatCanBeUsedThisEmojiAsReaction",
        ] {
            assert!(emoji.get(key).is_some(), "missing {key}");
        }
        assert!(emoji["host"].is_null());
        assert_eq!(emoji["type"], "image/png");
    }

    #[test]
    fn a_pack_zipped_as_a_folder_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("folder.zip");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("my-pack/meta.json", options).unwrap();
        zip.write_all(br#"{"emojis":[{"fileName":"a.png","downloaded":true,"emoji":{"name":"aa"}}]}"#).unwrap();
        zip.start_file("my-pack/a.png", options).unwrap();
        zip.write_all(b"image").unwrap();
        zip.start_file("my-pack/nested/b.png", options).unwrap();
        zip.write_all(b"other").unwrap();
        zip.finish().unwrap();

        let mut archive = Archive::open(&path).unwrap();
        assert_eq!(archive.meta(10).unwrap().entries[0].source_name, "aa");
        let sizes = archive.file_sizes();
        assert_eq!(sizes.get("a.png"), Some(&5));
        assert!(!sizes.contains_key("b.png") && !sizes.contains_key("nested/b.png"));
        assert_eq!(archive.extract("a.png", &dir.path().join("a.png"), 100), Ok(5));
    }

    #[test]
    fn files_that_are_no_pack_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let not_zip = dir.path().join("image.png");
        std::fs::write(&not_zip, b"\x89PNG\r\n\x1a\n not a zip").unwrap();
        assert!(matches!(Archive::open(&not_zip), Err(PackError::ArchiveInvalid)));

        let no_meta = dir.path().join("plain.zip");
        let mut zip = ZipWriter::new(File::create(&no_meta).unwrap());
        zip.start_file("a.png", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        zip.write_all(b"image").unwrap();
        zip.finish().unwrap();
        assert!(matches!(Archive::open(&no_meta), Err(PackError::MetaMissing)));
    }
}
