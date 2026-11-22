//! Text written into the data tables (rules, missions, badges): Japanese
//! first, English second, both known at compile time.
use tokuly_core::outbound::Localized;
use tokuly_proto::common::LocalizedText;

/// A piece of copy in both languages of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text {
    pub ja: &'static str,
    pub en: &'static str,
}

/// Shorthand for the data tables: `t("スタンプを送ろう", "Send a stamp")`.
pub const fn t(ja: &'static str, en: &'static str) -> Text {
    Text { ja, en }
}

impl Text {
    pub const EMPTY: Text = t("", "");

    pub fn is_empty(&self) -> bool {
        self.ja.is_empty() && self.en.is_empty()
    }

    /// The text as an API message.
    pub fn proto(&self) -> LocalizedText {
        LocalizedText { ja: self.ja.to_owned(), en: self.en.to_owned(), ..Default::default() }
    }

    /// The text as an optional API field: unset when there is none.
    pub fn proto_opt(&self) -> Option<LocalizedText> {
        (!self.is_empty()).then(|| self.proto())
    }

    /// The text for notifications and activity entries.
    pub fn localized(&self) -> Localized {
        Localized::new(self.ja, self.en)
    }

    /// The text with every `{n}` replaced by a number, as an API message.
    pub fn proto_with(&self, n: i64) -> LocalizedText {
        let n = group_digits(n);
        LocalizedText { ja: self.ja.replace("{n}", &n), en: self.en.replace("{n}", &n), ..Default::default() }
    }
}

/// `12345` → `12,345`.
pub fn group_digits(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Text in both languages built at run time.
pub fn localized_text(ja: impl Into<String>, en: impl Into<String>) -> LocalizedText {
    LocalizedText { ja: ja.into(), en: en.into(), ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_grouped_in_threes() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(group_digits(-12345), "-12,345");
    }

    #[test]
    fn placeholders_are_filled_in_both_languages() {
        let text = t("友達を{n}人追加する", "Add {n} friends").proto_with(1500);
        assert_eq!(text.ja, "友達を1,500人追加する");
        assert_eq!(text.en, "Add 1,500 friends");
    }

    #[test]
    fn empty_text_is_an_unset_field() {
        assert!(Text::EMPTY.proto_opt().is_none());
        assert!(t("a", "b").proto_opt().is_some());
    }
}
