//! 割り勘 arithmetic: dividing an expense, balances, and the transfers that
//! settle everyone.
//!
//! Everything here is integer arithmetic on minor units (yen, cents). Three
//! properties hold by construction and are checked by the property tests at
//! the bottom:
//!
//! 1. the shares of an expense always add up to the expense, whatever the
//!    remainder — it is handed out one minor unit at a time, in a fixed order;
//! 2. the balances of a plan always add up to zero;
//! 3. applying the transfers of [`settle`] brings every balance to zero.
//!
//! Nothing here touches the database or knows about plans: callers pass plain
//! numbers in and store what comes out.
use std::collections::BTreeMap;

use uuid::Uuid;

/// Divides `amount` into equal parts, one per person. What cannot be divided
/// goes, one minor unit each, to the first people in ascending order of ID.
/// `people` must hold each person once.
pub(crate) fn split_equal(amount: i64, people: &[Uuid]) -> Vec<(Uuid, i64)> {
    let weighted: Vec<(Uuid, i64)> = people.iter().map(|person| (*person, 1)).collect();
    split_by_weight(amount, &weighted)
}

/// Divides `amount` in proportion to each person's weight (largest-remainder
/// method): everyone first gets their share rounded down, then the minor
/// units left over go to the largest fractions, ties in ascending order of
/// ID. Weights must be positive and each person listed once.
pub(crate) fn split_by_weight(amount: i64, weights: &[(Uuid, i64)]) -> Vec<(Uuid, i64)> {
    let total: i128 = weights.iter().map(|(_, weight)| *weight as i128).sum();
    if total <= 0 || amount <= 0 {
        return weights.iter().map(|(person, _)| (*person, 0)).collect();
    }
    // (person, floor share, fraction numerator): the fraction is the part of
    // `amount * weight / total` lost by rounding down.
    let mut parts: Vec<(Uuid, i64, i128)> = weights
        .iter()
        .map(|(person, weight)| {
            let exact = amount as i128 * *weight as i128;
            (*person, (exact / total) as i64, exact % total)
        })
        .collect();
    let handed_out: i64 = parts.iter().map(|(_, share, _)| *share).sum();
    let mut left = amount - handed_out;

    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|a, b| parts[*b].2.cmp(&parts[*a].2).then(parts[*a].0.cmp(&parts[*b].0)));
    for index in order {
        if left == 0 {
            break;
        }
        parts[index].1 += 1;
        left -= 1;
    }
    parts.into_iter().map(|(person, share, _)| (person, share)).collect()
}

/// An expense as the arithmetic sees it.
#[derive(Debug, Clone)]
pub(crate) struct Paid {
    pub payer: Uuid,
    pub amount: i64,
    /// What each person owes for it; adds up to `amount`.
    pub shares: Vec<(Uuid, i64)>,
}

/// A transfer between two people: one that is suggested, or one already made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Transfer {
    pub from: Uuid,
    pub to: Uuid,
    pub amount: i64,
}

/// One person's position.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Position {
    pub paid: i64,
    pub owed: i64,
    pub sent: i64,
    pub received: i64,
}

impl Position {
    /// Positive: the others still owe this person. Negative: they owe.
    pub(crate) fn net(&self) -> i64 {
        self.paid - self.owed + self.sent - self.received
    }
}

/// Everyone's position after `expenses` and the transfers already `made`,
/// in ascending order of ID. The nets always add up to zero.
pub(crate) fn positions(expenses: &[Paid], made: &[Transfer]) -> BTreeMap<Uuid, Position> {
    let mut out: BTreeMap<Uuid, Position> = BTreeMap::new();
    for expense in expenses {
        out.entry(expense.payer).or_default().paid += expense.amount;
        for (person, share) in &expense.shares {
            out.entry(*person).or_default().owed += share;
        }
    }
    for transfer in made {
        out.entry(transfer.from).or_default().sent += transfer.amount;
        out.entry(transfer.to).or_default().received += transfer.amount;
    }
    out
}

/// Up to this many open balances the fewest possible transfers are found
/// exactly; beyond it [`settle`] falls back to the greedy pairing, which
/// needs at most one transfer more than the optimum per missed group.
const EXACT_LIMIT: usize = 16;

/// The transfers that bring every balance in `nets` to zero, as few as
/// possible ("AがBに1,200円").
///
/// `nets` holds each person's net position (positive = is owed money) and
/// must add up to zero. With `n` open balances `n - 1` transfers always
/// suffice; fewer are possible exactly when the people fall into groups that
/// are settled among themselves, so the task is to find the largest number of
/// such groups. That is done exactly (dynamic programming over subsets) for
/// up to [`EXACT_LIMIT`] open balances. The result is deterministic: it only
/// depends on the balances, not on their order.
pub(crate) fn settle(nets: &[(Uuid, i64)]) -> Vec<Transfer> {
    let mut open: Vec<(Uuid, i64)> = nets.iter().copied().filter(|(_, net)| *net != 0).collect();
    open.sort();
    debug_assert_eq!(open.iter().map(|(_, net)| *net as i128).sum::<i128>(), 0, "balances add up to zero");

    let groups = if open.len() <= EXACT_LIMIT { settled_groups(&open) } else { vec![open] };
    let mut transfers: Vec<Transfer> = groups.iter().flat_map(|group| pair_off(group)).collect();
    transfers.sort_by(|a, b| (a.from, a.to).cmp(&(b.from, b.to)));
    transfers
}

/// Splits the open balances into as many groups as possible that each add up
/// to zero.
fn settled_groups(open: &[(Uuid, i64)]) -> Vec<Vec<(Uuid, i64)>> {
    let n = open.len();
    if n == 0 {
        return Vec::new();
    }
    let full = (1usize << n) - 1;
    // sums[mask]: total of the balances in `mask`.
    // best[mask]: the most zero-sum groups a chain of subsets ending in
    // `mask` can pass through, `mask` itself included when it adds up to zero.
    let mut sums = vec![0i64; full + 1];
    let mut best = vec![0u8; full + 1];
    for mask in 1..=full {
        let lowest = mask.trailing_zeros() as usize;
        sums[mask] = sums[mask & (mask - 1)] + open[lowest].1;
        let mut most = 0;
        let mut rest = mask;
        while rest != 0 {
            let bit = rest & rest.wrapping_neg();
            most = most.max(best[mask ^ bit]);
            rest ^= bit;
        }
        best[mask] = most + u8::from(sums[mask] == 0);
    }

    // Walk back from everyone to nobody along a best chain; the people
    // dropped between two zero-sum subsets form one group.
    let mut groups = Vec::new();
    let mut current = Vec::new();
    let mut mask = full;
    while mask != 0 {
        let wanted = best[mask] - u8::from(sums[mask] == 0);
        let index =
            (0..n).find(|i| mask & (1 << i) != 0 && best[mask ^ (1 << i)] == wanted).expect("a best chain continues");
        current.push(open[index]);
        mask ^= 1 << index;
        if sums[mask] == 0 {
            groups.push(std::mem::take(&mut current));
        }
    }
    groups
}

/// Settles one zero-sum group by repeatedly letting whoever owes most pay
/// whoever is owed most. Every transfer clears at least one person and the
/// last one clears two, so a group of `k` needs at most `k - 1`.
fn pair_off(group: &[(Uuid, i64)]) -> Vec<Transfer> {
    let mut creditors: Vec<(Uuid, i64)> = group.iter().copied().filter(|(_, net)| *net > 0).collect();
    let mut debtors: Vec<(Uuid, i64)> =
        group.iter().filter(|(_, net)| *net < 0).map(|(person, net)| (*person, -net)).collect();
    let mut transfers = Vec::new();
    loop {
        // Largest amount first; the smaller ID wins a tie.
        creditors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        debtors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let (Some(creditor), Some(debtor)) = (creditors.first_mut(), debtors.first_mut()) else { break };
        let amount = creditor.1.min(debtor.1);
        transfers.push(Transfer { from: debtor.0, to: creditor.0, amount });
        creditor.1 -= amount;
        debtor.1 -= amount;
        creditors.retain(|(_, left)| *left > 0);
        debtors.retain(|(_, left)| *left > 0);
    }
    transfers
}

/// Digits after the decimal point of a currency's minor unit (ISO 4217).
pub(crate) fn minor_digits(currency: &str) -> u32 {
    match currency {
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF" | "UGX" | "VND" | "VUV"
        | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        _ => 2,
    }
}

/// An amount as people write it: "1,200円" for yen, "USD 12.50" otherwise.
pub(crate) fn format_money(amount: i64, currency: &str) -> String {
    let digits = minor_digits(currency);
    let unit = 10i64.pow(digits);
    let whole = group_thousands(amount / unit);
    let number =
        if digits == 0 { whole } else { format!("{whole}.{:0width$}", amount % unit, width = digits as usize) };
    if currency == "JPY" { format!("{number}円") } else { format!("{currency} {number}") }
}

fn group_thousands(value: i64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Person number `n`, with IDs ascending in `n`.
    fn person(n: u8) -> Uuid {
        Uuid::from_u128(n as u128 + 1)
    }

    fn apply(nets: &[(Uuid, i64)], transfers: &[Transfer]) -> BTreeMap<Uuid, i64> {
        let mut left: BTreeMap<Uuid, i64> = nets.iter().copied().collect();
        for transfer in transfers {
            // Paying raises the payer's net towards zero; receiving lowers the receiver's.
            *left.entry(transfer.from).or_default() += transfer.amount;
            *left.entry(transfer.to).or_default() -= transfer.amount;
        }
        left
    }

    #[test]
    fn equal_split_hands_the_remainder_out_in_id_order() {
        let people = [person(2), person(0), person(1)];
        let shares = split_equal(1000, &people);
        assert_eq!(shares, vec![(person(2), 333), (person(0), 334), (person(1), 333)]);
        assert_eq!(split_equal(1001, &people), vec![(person(2), 333), (person(0), 334), (person(1), 334)]);
        assert_eq!(split_equal(2, &people), vec![(person(2), 0), (person(0), 1), (person(1), 1)]);
        assert_eq!(split_equal(900, &people), vec![(person(2), 300), (person(0), 300), (person(1), 300)]);
    }

    #[test]
    fn weighted_split_gives_the_leftover_to_the_largest_fractions() {
        // 1000 at 2 : 1 : 1 divides evenly.
        assert_eq!(
            split_by_weight(1000, &[(person(0), 2), (person(1), 1), (person(2), 1)]),
            vec![(person(0), 500), (person(1), 250), (person(2), 250)]
        );
        // 100 at 1 : 1 : 1 leaves one unit: equal fractions, so the smallest ID gets it.
        assert_eq!(
            split_by_weight(100, &[(person(0), 1), (person(1), 1), (person(2), 1)]),
            vec![(person(0), 34), (person(1), 33), (person(2), 33)]
        );
        // 10 at 3 : 3 : 1 = 4.28 / 4.28 / 1.42: one unit left, the largest fraction is the last person's.
        assert_eq!(
            split_by_weight(10, &[(person(0), 3), (person(1), 3), (person(2), 1)]),
            vec![(person(0), 4), (person(1), 4), (person(2), 2)]
        );
    }

    #[test]
    fn positions_follow_who_paid_and_who_owes() {
        let (a, b, c) = (person(0), person(1), person(2));
        let dinner = Paid { payer: a, amount: 3000, shares: split_equal(3000, &[a, b, c]) };
        let taxi = Paid { payer: b, amount: 1200, shares: split_equal(1200, &[b, c]) };
        let all = positions(&[dinner, taxi], &[Transfer { from: c, to: a, amount: 500 }]);
        assert_eq!(all[&a], Position { paid: 3000, owed: 1000, sent: 0, received: 500 });
        assert_eq!(all[&b], Position { paid: 1200, owed: 1600, sent: 0, received: 0 });
        assert_eq!(all[&c], Position { paid: 0, owed: 1600, sent: 500, received: 0 });
        assert_eq!((all[&a].net(), all[&b].net(), all[&c].net()), (1500, -400, -1100));
    }

    #[test]
    fn settling_uses_one_transfer_per_person_but_the_last() {
        let (a, b, c) = (person(0), person(1), person(2));
        let transfers = settle(&[(a, 1500), (b, -400), (c, -1100)]);
        assert_eq!(
            transfers,
            vec![Transfer { from: b, to: a, amount: 400 }, Transfer { from: c, to: a, amount: 1100 }]
        );
        assert!(settle(&[(a, 0), (b, 0)]).is_empty());
        assert!(settle(&[]).is_empty());
    }

    #[test]
    fn settling_finds_groups_that_settle_among_themselves() {
        // Pairing largest with largest across all six would take five
        // transfers. The six fall into two groups that settle among
        // themselves, {9, -4, -5} and {8, -6, -2}: two transfers each.
        let people: Vec<Uuid> = (0..6).map(person).collect();
        let nets = [(people[0], 9), (people[1], 8), (people[2], -4), (people[3], -5), (people[4], -6), (people[5], -2)];
        let transfers = settle(&nets);
        assert_eq!(transfers.len(), 4);
        assert!(apply(&nets, &transfers).values().all(|left| *left == 0));
    }

    #[test]
    fn settling_does_not_depend_on_the_order_of_the_balances() {
        let nets = [(person(0), 700), (person(1), -300), (person(2), -150), (person(3), 50), (person(4), -300)];
        let mut reversed = nets;
        reversed.reverse();
        assert_eq!(settle(&nets), settle(&reversed));
    }

    #[test]
    fn large_groups_are_settled_greedily() {
        // 20 open balances: more than the exact search handles.
        let mut nets: Vec<(Uuid, i64)> = (0..19).map(|n| (person(n), -(n as i64 + 1) * 10)).collect();
        nets.push((person(19), 1900));
        let transfers = settle(&nets);
        assert_eq!(transfers.len(), 19);
        assert!(apply(&nets, &transfers).values().all(|left| *left == 0));
    }

    #[test]
    fn money_is_written_the_way_people_write_it() {
        assert_eq!(format_money(1200, "JPY"), "1,200円");
        assert_eq!(format_money(5, "JPY"), "5円");
        assert_eq!(format_money(1_234_567, "JPY"), "1,234,567円");
        assert_eq!(format_money(1250, "USD"), "USD 12.50");
        assert_eq!(format_money(100_005, "EUR"), "EUR 1,000.05");
        assert_eq!(format_money(1500, "KWD"), "KWD 1.500");
    }

    /// Distinct people out of a group of twelve.
    fn people(max: usize) -> impl Strategy<Value = Vec<Uuid>> {
        proptest::sample::subsequence((0u8..12).collect::<Vec<_>>(), 1..=max)
            .prop_map(|ns| ns.into_iter().map(person).collect())
    }

    /// An expense split equally, by weight or by exact amounts.
    fn expense() -> impl Strategy<Value = Paid> {
        (0u8..12, 1i64..=5_000_000, people(12), 0u8..3, proptest::collection::vec(1i64..=1000, 12)).prop_map(
            |(payer, amount, people, method, weights)| {
                let weighted: Vec<(Uuid, i64)> = people.iter().copied().zip(weights).collect();
                let shares = match method {
                    0 => split_equal(amount, &people),
                    1 => split_by_weight(amount, &weighted),
                    // "Exact" amounts as a client would send them: any numbers that add up.
                    _ => {
                        let mut shares = split_by_weight(amount, &weighted);
                        shares.reverse();
                        shares
                    }
                };
                Paid { payer: person(payer), amount, shares }
            },
        )
    }

    fn transfer() -> impl Strategy<Value = Transfer> {
        (0u8..12, 0u8..12, 1i64..=1_000_000)
            .prop_filter("two different people", |(from, to, _)| from != to)
            .prop_map(|(from, to, amount)| Transfer { from: person(from), to: person(to), amount })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn shares_add_up_and_stay_within_one_unit_of_the_exact_share(amount in 1i64..=1_000_000_000_000, weights in proptest::collection::vec(1i64..=1000, 1..=30)) {
            let weighted: Vec<(Uuid, i64)> = weights.iter().enumerate().map(|(n, weight)| (person(n as u8), *weight)).collect();
            let shares = split_by_weight(amount, &weighted);
            prop_assert_eq!(shares.iter().map(|(_, share)| *share).sum::<i64>(), amount);
            let total: i128 = weights.iter().map(|w| *w as i128).sum();
            for ((_, share), weight) in shares.iter().zip(&weights) {
                let floor = (amount as i128 * *weight as i128 / total) as i64;
                prop_assert!(*share == floor || *share == floor + 1, "share {share} is not the exact share {floor} rounded");
            }
        }

        #[test]
        fn equal_shares_differ_by_at_most_one_unit(amount in 1i64..=1_000_000_000_000, people in people(12)) {
            let shares = split_equal(amount, &people);
            prop_assert_eq!(shares.iter().map(|(_, share)| *share).sum::<i64>(), amount);
            let smallest = shares.iter().map(|(_, share)| *share).min().unwrap();
            let largest = shares.iter().map(|(_, share)| *share).max().unwrap();
            prop_assert!(largest - smallest <= 1);
        }

        #[test]
        fn balances_always_add_up_to_zero(expenses in proptest::collection::vec(expense(), 0..20), made in proptest::collection::vec(transfer(), 0..10)) {
            let all = positions(&expenses, &made);
            prop_assert_eq!(all.values().map(Position::net).sum::<i64>(), 0);
        }

        #[test]
        fn the_settlement_plan_clears_every_balance(expenses in proptest::collection::vec(expense(), 0..20), made in proptest::collection::vec(transfer(), 0..10)) {
            let nets: Vec<(Uuid, i64)> = positions(&expenses, &made).into_iter().map(|(person, position)| (person, position.net())).collect();
            let transfers = settle(&nets);
            for transfer in &transfers {
                prop_assert!(transfer.amount > 0);
                prop_assert_ne!(transfer.from, transfer.to);
            }
            prop_assert!(apply(&nets, &transfers).values().all(|left| *left == 0), "balances left after settling");
            let open = nets.iter().filter(|(_, net)| *net != 0).count();
            prop_assert!(transfers.len() <= open.saturating_sub(1), "{} transfers for {open} open balances", transfers.len());
        }

        #[test]
        fn pairs_that_cancel_out_need_one_transfer_each(amounts in proptest::collection::vec(1i64..=1_000_000, 1..=6)) {
            // Person 2k is owed what person 2k+1 owes: k transfers settle everything.
            let nets: Vec<(Uuid, i64)> = amounts.iter().enumerate().flat_map(|(k, amount)| [(person(2 * k as u8), *amount), (person(2 * k as u8 + 1), -amount)]).collect();
            let transfers = settle(&nets);
            prop_assert!(transfers.len() <= amounts.len(), "{} transfers for {} pairs", transfers.len(), amounts.len());
            prop_assert!(apply(&nets, &transfers).values().all(|left| *left == 0));
        }
    }
}
