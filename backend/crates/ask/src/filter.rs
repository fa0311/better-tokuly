//! The word filter: decides whether a text is held back from an inbox.
//!
//! A text is held when it contains a muted word, either from the server's
//! list or from the recipient's own. Held texts are not lost; they wait in
//! the FILTERED folder where the recipient can look when they feel like it.
//!
//! Matching is deliberately simple and forgiving in one direction only:
//! - both sides are normalized (NFKC, lower case), so full-width letters,
//!   half-width kana and capitals make no difference;
//! - a word written in Latin letters or digits matches whole words only
//!   ("ugly" does not match "smugly"), a phrase matches consecutive words;
//! - any other word (Japanese has no word boundaries) matches as a
//!   substring, ignoring spaces, punctuation and emoji placed between its
//!   characters ("死 ね" and "死・ね" match "死ね").
use unicode_normalization::UnicodeNormalization as _;

/// A list of muted words, normalized once.
#[derive(Debug, Clone, Default)]
pub(crate) struct WordFilter {
    /// Words matched against whole words: `" kill yourself "`.
    spaced: Vec<String>,
    /// Words matched as substrings of the squeezed text.
    squeezed: Vec<String>,
}

/// A text prepared for matching against any number of filters.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    /// The words of the text, separated and surrounded by single spaces.
    spaced: String,
    /// The text with everything but letters and digits removed.
    squeezed: String,
}

fn fold(text: &str) -> String {
    text.nfkc().flat_map(char::to_lowercase).collect()
}

/// The words of a folded text joined by single spaces, with one space at
/// each end, so that `contains(" word ")` is a whole-word test.
fn spaced(folded: &str) -> String {
    let mut out = String::with_capacity(folded.len() + 2);
    out.push(' ');
    for word in folded.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()) {
        out.push_str(word);
        out.push(' ');
    }
    out
}

fn squeezed(folded: &str) -> String {
    folded.chars().filter(|c| c.is_alphanumeric()).collect()
}

pub(crate) fn prepare(text: &str) -> Prepared {
    let folded = fold(text);
    Prepared { spaced: spaced(&folded), squeezed: squeezed(&folded) }
}

impl WordFilter {
    pub(crate) fn new<S: AsRef<str>>(words: impl IntoIterator<Item = S>) -> Self {
        let mut filter = Self::default();
        for word in words {
            let folded = fold(word.as_ref());
            let squeezed_word = squeezed(&folded);
            if squeezed_word.is_empty() {
                continue;
            }
            if squeezed_word.is_ascii() {
                filter.spaced.push(spaced(&folded));
            } else {
                filter.squeezed.push(squeezed_word);
            }
        }
        filter
    }

    pub(crate) fn matches(&self, text: &Prepared) -> bool {
        self.spaced.iter().any(|word| text.spaced.contains(word.as_str()))
            || self.squeezed.iter().any(|word| text.squeezed.contains(word.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(words: &[&str], text: &str) -> bool {
        WordFilter::new(words).matches(&prepare(text))
    }

    #[test]
    fn latin_words_match_whole_words_only() {
        assert!(held(&["ugly"], "you are UGLY!"));
        assert!(held(&["ugly"], "ugly"));
        assert!(!held(&["ugly"], "she smiled smugly"));
        assert!(!held(&["kys"], "skys the limit"));
        assert!(held(&["kys"], "just kys."));
    }

    #[test]
    fn latin_phrases_match_consecutive_words() {
        assert!(held(&["kill yourself"], "Kill   yourself, ok?"));
        assert!(held(&["Kill Yourself"], "kill-yourself"));
        assert!(!held(&["kill yourself"], "kill it yourself"));
        assert!(!held(&["go die"], "let's go diet together"));
    }

    #[test]
    fn japanese_words_match_as_substrings() {
        assert!(held(&["死ね"], "お前なんか死ねばいいのに"));
        assert!(held(&["うざい"], "まじでうざいんだけど"));
        assert!(!held(&["死ね"], "テストで死にそう"));
    }

    #[test]
    fn separators_between_characters_do_not_hide_a_word() {
        assert!(held(&["死ね"], "死 ね"));
        assert!(held(&["死ね"], "死・ね!!"));
        assert!(held(&["死ね"], "死🔥ね"));
    }

    #[test]
    fn width_and_case_are_folded() {
        // Full-width Latin letters and half-width katakana.
        assert!(held(&["ugly"], "ＵＧＬＹ"));
        assert!(held(&["キモい"], "ｷﾓい"));
        assert!(held(&["ＫＹＳ"], "kys"));
    }

    #[test]
    fn empty_filters_and_empty_words_match_nothing() {
        assert!(!held(&[], "anything"));
        assert!(!held(&["", "  ", "!!!"], "anything !!!"));
        assert!(!WordFilter::default().matches(&prepare("")));
    }

    #[test]
    fn the_default_list_holds_abuse_and_passes_ordinary_questions() {
        let words = crate::config::AskConfig::default();
        let filter = WordFilter::new(words.filter_word_list());
        for abusive in ["死ねよ", "まじキモい", "kys", "You are so ugly", "nobody likes you lol", "住所教えてよ"]
        {
            assert!(filter.matches(&prepare(abusive)), "{abusive}");
        }
        for fine in [
            "好きな食べ物は?",
            "部活なにやってる?",
            "What's your favourite subject?",
            "テスト勉強してる?",
            "今日の放課後ひま?",
        ] {
            assert!(!filter.matches(&prepare(fine)), "{fine}");
        }
    }
}
