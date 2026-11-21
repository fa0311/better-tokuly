//! Article tags: normalization and keyword tagging.
//!
//! An article's tags come from three places: the tags its source was given
//! by an admin, the categories the feed itself assigns, and a keyword
//! dictionary applied to the title and summary. All of them go through
//! [`normalize_tag`], so that readers' interests (which are kept per tag)
//! do not split over spelling variants.
use unicode_normalization::UnicodeNormalization as _;

/// Longest tag, in characters.
pub const MAX_TAG_CHARS: usize = 40;
/// Most tags one article carries.
pub const MAX_ARTICLE_TAGS: usize = 12;

/// The built-in dictionary (see [`TagDictionary::parse`] for the format).
/// Tags are Japanese first, like the rest of the product; keywords cover
/// both languages.
const BUILTIN: &str = "\
テクノロジー=ai|人工知能|生成ai|スマホ|スマートフォン|iphone|android|アプリ|ガジェット|半導体|ソフトウェア|クラウド|セキュリティ|プログラミング|ロボット|technology|software|startup;\
ゲーム=ゲーム|任天堂|nintendo|switch|playstation|ps5|xbox|steam|eスポーツ|esports|gaming;\
アニメ・漫画=アニメ|漫画|マンガ|コミック|声優|anime|manga;\
音楽=音楽|アルバム|新曲|シングル|ミュージック|music|album;\
映画・ドラマ=映画|ドラマ|俳優|女優|netflix|movie|film;\
スポーツ=スポーツ|野球|サッカー|バスケ|テニス|ゴルフ|五輪|オリンピック|jリーグ|プロ野球|sports|soccer|baseball;\
エンタメ=芸能|アイドル|お笑い|タレント|youtuber|vtuber|エンタメ|entertainment;\
ビジネス=経済|株価|企業|決算|投資|円安|円高|ビジネス|市場|business|economy;\
政治=政治|選挙|国会|首相|政府|内閣|法案|politics|election;\
国際=海外|国際|米国|中国|欧州|world;\
科学=科学|宇宙|研究|ロケット|nasa|jaxa|医学|science|space;\
グルメ=グルメ|料理|レシピ|ラーメン|カフェ|スイーツ|レストラン|food|recipe;\
旅行・おでかけ=旅行|観光|ホテル|温泉|おでかけ|travel;\
ファッション・美容=ファッション|コスメ|美容|メイク|fashion|beauty;\
暮らし=天気|防災|地震|台風|暮らし|健康|weather|health";

/// Text in the form tags and keywords are compared in: NFKC (full-width
/// letters and half-width kana fold to their usual forms), lowercase.
fn fold(text: &str) -> String {
    text.nfkc().collect::<String>().to_lowercase()
}

/// A tag as stored: folded, single spaces, no leading `#`, 1 to
/// [`MAX_TAG_CHARS`] characters. `None` when nothing usable is left.
pub fn normalize_tag(raw: &str) -> Option<String> {
    let folded = fold(raw);
    let cleaned: String = folded.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let tag = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let tag = tag.trim_start_matches('#').trim();
    let usable = !tag.is_empty() && tag.chars().count() <= MAX_TAG_CHARS && !tag.contains([',', ';', '|', '=']);
    usable.then(|| tag.to_owned())
}

/// Normalizes, de-duplicates (keeping the first occurrence) and caps tags.
pub fn merge_tags<I, S>(tags: I, max: usize) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut merged: Vec<String> = Vec::new();
    for tag in tags {
        if merged.len() >= max {
            break;
        }
        if let Some(tag) = normalize_tag(tag.as_ref())
            && !merged.contains(&tag)
        {
            merged.push(tag);
        }
    }
    merged
}

/// Keywords that give an article a tag.
#[derive(Debug, Clone, Default)]
pub struct TagDictionary {
    /// `(tag, folded keywords)`, in the order of the specification.
    entries: Vec<(String, Vec<String>)>,
}

impl TagDictionary {
    /// The dictionary `TOKULY_NEWS_TAG_KEYWORDS` describes: empty means the
    /// built-in one, `-` means none.
    pub fn from_config(spec: &str) -> anyhow::Result<Self> {
        match spec.trim() {
            "" => Self::parse(BUILTIN),
            "-" => Ok(Self::default()),
            spec => Self::parse(spec),
        }
    }

    /// Parses `tag=keyword|keyword;tag=keyword...` (entries may also be
    /// separated by line breaks).
    pub fn parse(spec: &str) -> anyhow::Result<Self> {
        let mut entries: Vec<(String, Vec<String>)> = Vec::new();
        for entry in spec.split([';', '\n']).map(str::trim).filter(|entry| !entry.is_empty()) {
            let Some((tag, keywords)) = entry.split_once('=') else {
                anyhow::bail!("news tag dictionary: {entry:?} is not `tag=keyword|keyword`");
            };
            let Some(tag) = normalize_tag(tag) else {
                anyhow::bail!("news tag dictionary: {tag:?} is not a usable tag");
            };
            let keywords: Vec<String> =
                keywords.split('|').map(|keyword| fold(keyword.trim())).filter(|keyword| !keyword.is_empty()).collect();
            anyhow::ensure!(!keywords.is_empty(), "news tag dictionary: tag {tag:?} has no keywords");
            match entries.iter_mut().find(|(existing, _)| *existing == tag) {
                Some((_, existing)) => existing.extend(keywords),
                None => entries.push((tag, keywords)),
            }
        }
        Ok(Self { entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Tags whose keywords occur in `text`, in dictionary order.
    pub fn tags_for(&self, text: &str) -> Vec<String> {
        let folded = fold(text);
        self.entries
            .iter()
            .filter(|(_, keywords)| keywords.iter().any(|keyword| contains_keyword(&folded, keyword)))
            .map(|(tag, _)| tag.clone())
            .collect()
    }
}

/// Whether `text` contains `keyword`. A keyword written in ASCII letters or
/// digits must stand alone as a word ("ai" is not found in "rain"); anything
/// else (Japanese has no word boundaries) matches anywhere.
fn contains_keyword(text: &str, keyword: &str) -> bool {
    let wordy = |c: char| c.is_ascii_alphanumeric();
    let starts_wordy = keyword.chars().next().is_some_and(wordy);
    let ends_wordy = keyword.chars().next_back().is_some_and(wordy);
    let mut from = 0;
    while let Some(found) = text[from..].find(keyword) {
        let (start, end) = (from + found, from + found + keyword.len());
        let free_before = !starts_wordy || !text[..start].chars().next_back().is_some_and(wordy);
        let free_after = !ends_wordy || !text[end..].chars().next().is_some_and(wordy);
        if free_before && free_after {
            return true;
        }
        // Step one character on; `start` is a character boundary.
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_folded_and_tidied() {
        assert_eq!(normalize_tag("  Tech  News ").as_deref(), Some("tech news"));
        assert_eq!(normalize_tag("#ゲーム").as_deref(), Some("ゲーム"));
        // Full-width letters and half-width kana are the same tag as their usual forms.
        assert_eq!(normalize_tag("ＡＩ").as_deref(), Some("ai"));
        assert_eq!(normalize_tag("ｹﾞｰﾑ").as_deref(), Some("ゲーム"));
        assert_eq!(normalize_tag("line\nbreak").as_deref(), Some("line break"));
    }

    #[test]
    fn unusable_tags_are_rejected() {
        for raw in ["", "   ", "#", "a,b", "a;b", "a|b", "a=b"] {
            assert_eq!(normalize_tag(raw), None, "{raw:?}");
        }
        assert_eq!(normalize_tag(&"あ".repeat(MAX_TAG_CHARS)).map(|t| t.chars().count()), Some(MAX_TAG_CHARS));
        assert_eq!(normalize_tag(&"あ".repeat(MAX_TAG_CHARS + 1)), None);
    }

    #[test]
    fn merging_keeps_first_occurrences_up_to_the_cap() {
        let merged = merge_tags(["Tech", "ゲーム", "tech", "", "ＴＥＣＨ", "音楽"], 10);
        assert_eq!(merged, vec!["tech", "ゲーム", "音楽"]);
        assert_eq!(merge_tags(["a", "b", "c", "d"], 2), vec!["a", "b"]);
        assert!(merge_tags(Vec::<String>::new(), 5).is_empty());
    }

    #[test]
    fn the_dictionary_format_is_parsed_and_validated() {
        let dictionary = TagDictionary::parse("Tech = AI | 人工知能 ;ゲーム=switch\nTech=robot").unwrap();
        assert_eq!(dictionary.entries.len(), 2);
        assert_eq!(
            dictionary.entries[0],
            ("tech".to_owned(), vec!["ai".to_owned(), "人工知能".to_owned(), "robot".to_owned()])
        );
        assert!(TagDictionary::parse("no-equals-sign").is_err());
        assert!(TagDictionary::parse("=keyword").is_err());
        assert!(TagDictionary::parse("tag=").is_err());
        assert!(TagDictionary::parse("tag= | ").is_err());
        assert!(TagDictionary::parse("  ").unwrap().is_empty());
    }

    #[test]
    fn configuration_selects_builtin_custom_or_no_dictionary() {
        let builtin = TagDictionary::from_config("").unwrap();
        assert!(builtin.entries.len() >= 10, "the built-in dictionary parses");
        assert!(TagDictionary::from_config(" - ").unwrap().is_empty());
        assert_eq!(TagDictionary::from_config("pets=cat|dog").unwrap().tags_for("My dog"), vec!["pets"]);
        assert!(TagDictionary::from_config("broken").is_err());
    }

    #[test]
    fn ascii_keywords_match_whole_words_only() {
        let dictionary = TagDictionary::parse("tech=ai|switch 2").unwrap();
        for text in ["AI is everywhere", "The new AI.", "生成AIの時代", "ＡＩ特集", "(ai)", "Switch 2 review"]
        {
            assert_eq!(dictionary.tags_for(text), vec!["tech"], "{text}");
        }
        for text in ["It may rain in Spain", "Kaiju attack", "aint", "switch 20 ports", ""] {
            assert!(dictionary.tags_for(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn japanese_keywords_match_anywhere() {
        let dictionary = TagDictionary::parse("ゲーム=ゲーム|任天堂;音楽=新曲").unwrap();
        assert_eq!(dictionary.tags_for("人気ゲームの続編が発表"), vec!["ゲーム"]);
        assert_eq!(dictionary.tags_for("任天堂が新曲を公開"), vec!["ゲーム", "音楽"]);
        assert_eq!(dictionary.tags_for("ｹﾞｰﾑ実況"), vec!["ゲーム"], "half-width kana fold to the same word");
        assert!(dictionary.tags_for("天気予報").is_empty());
    }

    #[test]
    fn the_builtin_dictionary_tags_typical_headlines() {
        let dictionary = TagDictionary::from_config("").unwrap();
        assert!(dictionary.tags_for("任天堂、Switch向け新作ゲームを発表").contains(&"ゲーム".to_owned()));
        assert!(dictionary.tags_for("New AI model released by startup").contains(&"テクノロジー".to_owned()));
        assert!(dictionary.tags_for("首相が国会で答弁").contains(&"政治".to_owned()));
        assert!(dictionary.tags_for("ただの日記です").is_empty());
    }
}
