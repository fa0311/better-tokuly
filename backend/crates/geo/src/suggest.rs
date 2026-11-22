//! Place suggestions from stay patterns: where the user keeps staying
//! without having a place there, and what that spot probably is (home after
//! several nights, work or school after several weekdays). Pure.
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use tokuly_proto::geo::PlaceLabel;

use crate::{
    detect::{is_workday_stay, night_of},
    math::{self, Point},
};

/// Stays this close to each other are the same spot.
pub const CLUSTER_RADIUS_M: f64 = 120.0;
/// Stays at the same spot before it is suggested as a plain place.
const MIN_VISITS: usize = 3;
/// Weekdays at the same spot before it is suggested as work or school.
const MIN_WORKDAYS: usize = 3;

/// A past stay that matched no place.
#[derive(Debug, Clone, Copy)]
pub struct Stay {
    pub position: Point,
    pub arrived: DateTime<Utc>,
    pub departed: DateTime<Utc>,
}

/// Stays grouped around one spot.
#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    pub center: Point,
    pub visit_count: usize,
    pub total_seconds: i64,
    /// Distinct nights spent there.
    pub nights: usize,
    /// Distinct weekdays spent there during working hours.
    pub workdays: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub cluster: Cluster,
    pub label: PlaceLabel,
}

/// What the user already has, and how eager to be.
#[derive(Debug, Clone, Copy)]
pub struct Rules {
    pub has_home: bool,
    pub has_work: bool,
    pub has_school: bool,
    /// Suggest school rather than work.
    pub student: bool,
    pub home_nights: usize,
}

/// Groups stays by proximity (each stay joins the first cluster whose center
/// is within [`CLUSTER_RADIUS_M`], oldest stays first) and counts nights and
/// workdays in the local time of each cluster.
pub fn clusters(stays: &[Stay], zone_at: impl Fn(Point) -> Tz) -> Vec<Cluster> {
    let mut sorted: Vec<&Stay> = stays.iter().filter(|stay| stay.departed > stay.arrived).collect();
    sorted.sort_by_key(|stay| stay.arrived);
    let mut groups: Vec<(Point, Vec<&Stay>)> = Vec::new();
    for stay in sorted {
        match groups.iter_mut().find(|(center, _)| math::distance(*center, stay.position) <= CLUSTER_RADIUS_M) {
            Some((center, members)) => {
                // Running mean keeps the center where the stays actually are.
                let weight = members.len() as f64 + 1.0;
                center.0 += (stay.position.0 - center.0) / weight;
                center.1 += (stay.position.1 - center.1) / weight;
                members.push(stay);
            }
            None => groups.push((stay.position, vec![stay])),
        }
    }
    groups
        .into_iter()
        .map(|(center, members)| {
            let zone = zone_at(center);
            let mut nights: Vec<NaiveDate> = members
                .iter()
                .filter_map(|stay| night_of(stay.arrived, stay.departed, zone))
                .map(|night| night.date)
                .collect();
            nights.sort();
            nights.dedup();
            let mut workdays: Vec<NaiveDate> = members
                .iter()
                .filter(|stay| is_workday_stay(stay.arrived, stay.departed, zone))
                .map(|stay| stay.arrived.with_timezone(&zone).date_naive())
                .collect();
            workdays.sort();
            workdays.dedup();
            Cluster {
                center,
                visit_count: members.len(),
                total_seconds: members.iter().map(|stay| (stay.departed - stay.arrived).num_seconds()).sum(),
                nights: nights.len(),
                workdays: workdays.len(),
            }
        })
        .collect()
}

/// What a cluster should be suggested as, if anything.
pub fn label_for(cluster: &Cluster, rules: Rules) -> Option<PlaceLabel> {
    if !rules.has_home && cluster.nights >= rules.home_nights {
        return Some(PlaceLabel::PLACE_LABEL_HOME);
    }
    if cluster.workdays >= MIN_WORKDAYS {
        if rules.student && !rules.has_school {
            return Some(PlaceLabel::PLACE_LABEL_SCHOOL);
        }
        if !rules.student && !rules.has_work {
            return Some(PlaceLabel::PLACE_LABEL_WORK);
        }
    }
    (cluster.visit_count >= MIN_VISITS).then_some(PlaceLabel::PLACE_LABEL_CUSTOM)
}

/// Suggestions for a user's unmatched stays: home first, then work or
/// school, then other frequent spots by time spent. Each of home, work and
/// school is suggested at most once (the strongest candidate).
pub fn suggestions(stays: &[Stay], rules: Rules, zone_at: impl Fn(Point) -> Tz, limit: usize) -> Vec<Suggestion> {
    let mut found: Vec<Suggestion> = clusters(stays, zone_at)
        .into_iter()
        .filter_map(|cluster| label_for(&cluster, rules).map(|label| Suggestion { cluster, label }))
        .collect();
    let priority = |label: PlaceLabel| match label {
        PlaceLabel::PLACE_LABEL_HOME => 0,
        PlaceLabel::PLACE_LABEL_WORK | PlaceLabel::PLACE_LABEL_SCHOOL => 1,
        _ => 2,
    };
    found.sort_by(|a, b| {
        priority(a.label)
            .cmp(&priority(b.label))
            .then_with(|| b.cluster.nights.cmp(&a.cluster.nights))
            .then_with(|| b.cluster.total_seconds.cmp(&a.cluster.total_seconds))
    });
    let mut seen_roles = Vec::new();
    for suggestion in &mut found {
        if suggestion.label != PlaceLabel::PLACE_LABEL_CUSTOM {
            if seen_roles.contains(&suggestion.label) {
                // A weaker candidate for a role already suggested is just a frequent spot.
                suggestion.label = PlaceLabel::PLACE_LABEL_CUSTOM;
            } else {
                seen_roles.push(suggestion.label);
            }
        }
    }
    found.retain(|s| s.label != PlaceLabel::PLACE_LABEL_CUSTOM || s.cluster.visit_count >= MIN_VISITS);
    found.truncate(limit);
    found
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use chrono_tz::Asia::Tokyo;

    use super::*;

    const HOME: Point = (35.6595, 139.7005);
    const OFFICE: Point = (35.6812, 139.7671);
    const CAFE: Point = (35.6700, 139.7300);
    const RULES: Rules = Rules { has_home: false, has_work: false, has_school: false, student: false, home_nights: 3 };

    fn jst(day: u32, hour: u32) -> DateTime<Utc> {
        Tokyo.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap().with_timezone(&Utc)
    }

    fn stay(position: Point, from: DateTime<Utc>, to: DateTime<Utc>) -> Stay {
        Stay { position, arrived: from, departed: to }
    }

    /// A working week (Mon 16th – Fri 20th of November 2026): nights at home, days at the office.
    fn working_week() -> Vec<Stay> {
        let mut stays = Vec::new();
        for day in 16..=20 {
            // GPS puts home a few meters apart every evening.
            let home = math::destination(HOME.0, HOME.1, (day * 40) as f64, 15.0);
            stays.push(stay(home, jst(day - 1, 20), jst(day, 8)));
            stays.push(stay(OFFICE, jst(day, 9), jst(day, 18)));
        }
        stays
    }

    #[test]
    fn stays_at_the_same_spot_form_one_cluster() {
        let found = clusters(&working_week(), |_| Tokyo);
        assert_eq!(found.len(), 2);
        let home = found.iter().find(|c| math::distance(c.center, HOME) < 30.0).expect("home cluster");
        assert_eq!((home.visit_count, home.nights, home.workdays), (5, 5, 0));
        assert_eq!(home.total_seconds, 5 * 12 * 3600);
        let office = found.iter().find(|c| math::distance(c.center, OFFICE) < 30.0).expect("office cluster");
        assert_eq!((office.visit_count, office.nights, office.workdays), (5, 0, 5));
    }

    #[test]
    fn nights_suggest_home_and_weekdays_suggest_work() {
        let found = suggestions(&working_week(), RULES, |_| Tokyo, 10);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].label, PlaceLabel::PLACE_LABEL_HOME);
        assert!(math::distance(found[0].cluster.center, HOME) < 30.0);
        assert_eq!(found[1].label, PlaceLabel::PLACE_LABEL_WORK);
    }

    #[test]
    fn students_get_school_instead_of_work() {
        let found = suggestions(&working_week(), Rules { student: true, ..RULES }, |_| Tokyo, 10);
        assert_eq!(found[1].label, PlaceLabel::PLACE_LABEL_SCHOOL);
    }

    #[test]
    fn roles_the_user_already_has_are_not_suggested_again() {
        let found = suggestions(&working_week(), Rules { has_home: true, has_work: true, ..RULES }, |_| Tokyo, 10);
        // Both spots are still frequent stays without a place: plain suggestions.
        assert_eq!(found.iter().map(|s| s.label).collect::<Vec<_>>(), vec![PlaceLabel::PLACE_LABEL_CUSTOM; 2]);
    }

    #[test]
    fn two_nights_are_not_a_home_yet() {
        let stays = vec![stay(HOME, jst(16, 21), jst(17, 7)), stay(HOME, jst(17, 21), jst(18, 7))];
        assert!(suggestions(&stays, RULES, |_| Tokyo, 10).is_empty());
        let eager = Rules { home_nights: 2, ..RULES };
        assert_eq!(suggestions(&stays, eager, |_| Tokyo, 10)[0].label, PlaceLabel::PLACE_LABEL_HOME);
    }

    #[test]
    fn frequent_spots_are_plain_suggestions_and_rare_ones_none() {
        let mut stays = working_week();
        for day in [16, 18, 20] {
            stays.push(stay(CAFE, jst(day, 18), jst(day, 19)));
        }
        // One evening somewhere else.
        stays.push(stay((35.70, 139.80), jst(19, 19), jst(19, 21)));
        let found = suggestions(&stays, RULES, |_| Tokyo, 10);
        assert_eq!(found.len(), 3);
        assert_eq!(found[2].label, PlaceLabel::PLACE_LABEL_CUSTOM);
        assert!(math::distance(found[2].cluster.center, CAFE) < 1.0);
        assert_eq!((found[2].cluster.visit_count, found[2].cluster.total_seconds), (3, 3 * 3600));
    }

    #[test]
    fn only_the_strongest_candidate_gets_a_role() {
        // Five nights at home, three at a partner's place.
        let mut stays = working_week();
        let partner = (35.72, 139.65);
        for day in 21..=23 {
            stays.push(stay(partner, jst(day, 21), jst(day + 1, 8)));
        }
        let found = suggestions(&stays, RULES, |_| Tokyo, 10);
        let homes: Vec<&Suggestion> = found.iter().filter(|s| s.label == PlaceLabel::PLACE_LABEL_HOME).collect();
        assert_eq!(homes.len(), 1);
        assert!(math::distance(homes[0].cluster.center, HOME) < 30.0);
        let other = found.iter().find(|s| math::distance(s.cluster.center, partner) < 1.0).unwrap();
        assert_eq!(other.label, PlaceLabel::PLACE_LABEL_CUSTOM);
    }

    #[test]
    fn nights_are_counted_in_the_local_time_of_the_spot() {
        // 12:00–21:00 UTC is 21:00–06:00 in Tokyo: a night there, an afternoon in London.
        let stays: Vec<Stay> = (16..19)
            .map(|day| {
                stay(
                    HOME,
                    Utc.with_ymd_and_hms(2026, 11, day, 12, 0, 0).unwrap(),
                    Utc.with_ymd_and_hms(2026, 11, day, 21, 0, 0).unwrap(),
                )
            })
            .collect();
        assert_eq!(clusters(&stays, |_| Tokyo)[0].nights, 3);
        assert_eq!(clusters(&stays, |_| chrono_tz::Europe::London)[0].nights, 0);
    }

    #[test]
    fn the_limit_and_degenerate_stays_are_handled() {
        assert!(suggestions(&working_week(), RULES, |_| Tokyo, 0).is_empty());
        assert_eq!(suggestions(&working_week(), RULES, |_| Tokyo, 1).len(), 1);
        let backwards = vec![stay(HOME, jst(17, 8), jst(16, 20))];
        assert!(clusters(&backwards, |_| Tokyo).is_empty());
    }
}
