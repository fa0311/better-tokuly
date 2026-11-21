//! Money arithmetic and display for payouts.
//!
//! Money is always an integer count of a currency's minor unit; nothing here
//! touches floating point.

/// ISO 4217 currencies that have no minor unit (1 unit = 1 "minor unit").
const ZERO_DECIMAL: &[&str] = &["BIF", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG", "RWF", "UGX", "VND", "VUV", "XAF", "XOF", "XPF"];

/// Money owed for `coins` earned coins at `minor_per_100_coins`, rounded down
/// (a fraction of a minor unit cannot be paid).
pub fn payout_minor(coins: i64, minor_per_100_coins: i64) -> i64 {
    let minor = i128::from(coins) * i128::from(minor_per_100_coins) / 100;
    i64::try_from(minor).unwrap_or(i64::MAX)
}

/// An amount for people to read: "¥1,000", "$12.34", "12.34 CHF".
pub fn format_money(minor: i64, currency: &str) -> String {
    let negative = minor < 0;
    let minor = minor.unsigned_abs();
    let digits = if ZERO_DECIMAL.contains(&currency) {
        group_thousands(minor)
    } else {
        format!("{}.{:02}", group_thousands(minor / 100), minor % 100)
    };
    let sign = if negative { "-" } else { "" };
    match currency {
        "JPY" => format!("{sign}¥{digits}"),
        "USD" => format!("{sign}${digits}"),
        "EUR" => format!("{sign}€{digits}"),
        _ => format!("{sign}{digits} {currency}"),
    }
}

/// A coin amount for people to read: "12,500".
pub fn format_coins(coins: i64) -> String {
    let sign = if coins < 0 { "-" } else { "" };
    format!("{sign}{}", group_thousands(coins.unsigned_abs()))
}

fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payout_amount_rounds_down_and_never_overflows() {
        assert_eq!(payout_minor(1000, 100), 1000);
        assert_eq!(payout_minor(1050, 50), 525);
        assert_eq!(payout_minor(199, 1), 1, "1.99 minor units pay 1");
        assert_eq!(payout_minor(99, 1), 0);
        assert_eq!(payout_minor(i64::MAX, i64::MAX), i64::MAX);
    }

    #[test]
    fn money_is_formatted_per_currency() {
        assert_eq!(format_money(1000, "JPY"), "¥1,000");
        assert_eq!(format_money(1234567, "JPY"), "¥1,234,567");
        assert_eq!(format_money(1234, "USD"), "$12.34");
        assert_eq!(format_money(5, "EUR"), "€0.05");
        assert_eq!(format_money(123456, "CHF"), "1,234.56 CHF");
        assert_eq!(format_money(5000, "KRW"), "5,000 KRW");
        assert_eq!(format_money(-250, "USD"), "-$2.50");
    }

    #[test]
    fn coins_are_grouped() {
        assert_eq!(format_coins(0), "0");
        assert_eq!(format_coins(999), "999");
        assert_eq!(format_coins(12500), "12,500");
        assert_eq!(format_coins(-1000), "-1,000");
    }
}
