//! How one bot moves: stay somewhere for a while, walk (or cycle) to the
//! next place along the streets, stay again.
use crate::{
    rng::Rng,
    town::{Point, Town, along, length},
};

/// Seconds between position fixes while moving, and while staying.
const MOVING_FIX_SECONDS: f64 = 5.0;
const STAYING_FIX_SECONDS: f64 = 30.0;
/// How far a fix may be from where the bot really is, like a phone's GPS.
const JITTER_M: f64 = 2.5;

/// What a walker did during a step.
#[derive(Debug, Clone, PartialEq)]
pub enum Moved {
    /// Where the bot's phone says it is.
    Fix(Fix),
    /// Set off towards a spot; `leg` numbers the bot's journeys.
    Departed {
        leg: u32,
        from: Point,
        to: Point,
    },
    Arrived {
        spot: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    pub at: Point,
    pub speed_mps: f64,
    /// Degrees clockwise from north; unset while staying.
    pub heading: Option<f64>,
    pub battery_percent: i32,
}

#[derive(Debug, Clone)]
enum Doing {
    Staying { spot: usize, until: f64 },
    Moving { to: usize, path: Vec<Point>, walked_m: f64 },
}

#[derive(Debug, Clone)]
pub struct Walker {
    rng: Rng,
    speed_mps: f64,
    doing: Doing,
    at: Point,
    leg: u32,
    next_fix: f64,
    battery: f64,
}

impl Walker {
    /// A walker waiting at a spot of its own choice, about to leave.
    pub fn new(mut rng: Rng, town: &Town, cycling: bool) -> Self {
        let spot = rng.below(town.spot_count());
        let speed_mps = if cycling { rng.between(3.6, 4.8) } else { rng.between(1.1, 1.6) };
        let until = rng.between(5.0, 120.0);
        let battery = rng.between(35.0, 100.0);
        Self {
            speed_mps,
            doing: Doing::Staying { spot, until },
            at: town.point(town.spot_corner(spot)),
            leg: 0,
            next_fix: 0.0,
            battery,
            rng,
        }
    }

    pub fn speed_mps(&self) -> f64 {
        self.speed_mps
    }

    /// The spot the bot is at, when it is staying somewhere.
    pub fn staying_at(&self) -> Option<usize> {
        match self.doing {
            Doing::Staying { spot, .. } => Some(spot),
            Doing::Moving { .. } => None,
        }
    }

    /// Moves the bot from `now - seconds` to `now`.
    pub fn advance(&mut self, town: &Town, now: f64, seconds: f64) -> Vec<Moved> {
        let mut moved = Vec::new();
        // A phone loses a percent every few minutes; at 15% it gets plugged in.
        self.battery -= seconds / 240.0;
        if self.battery < 15.0 {
            self.battery = 95.0;
        }
        match &mut self.doing {
            Doing::Staying { spot, until } if now >= *until => {
                let from = *spot;
                let to = (from + 1 + self.rng.below(town.spot_count() - 1)) % town.spot_count();
                let path = town.route(town.spot_corner(from), town.spot_corner(to), &mut self.rng);
                self.leg += 1;
                moved.push(Moved::Departed {
                    leg: self.leg,
                    from: path[0],
                    to: *path.last().expect("a route has an end"),
                });
                self.doing = Doing::Moving { to, path, walked_m: 0.0 };
                self.next_fix = now;
            }
            Doing::Staying { .. } => {}
            Doing::Moving { to, path, walked_m } => {
                *walked_m += self.speed_mps * seconds;
                self.at = along(path, *walked_m).0;
                if *walked_m >= length(path) {
                    let spot = *to;
                    self.doing = Doing::Staying { spot, until: now + self.rng.between(180.0, 600.0) };
                    self.next_fix = now;
                    moved.push(Moved::Arrived { spot });
                }
            }
        }
        if now >= self.next_fix {
            moved.push(Moved::Fix(self.fix()));
            let moving = matches!(self.doing, Doing::Moving { .. });
            self.next_fix = now + if moving { MOVING_FIX_SECONDS } else { STAYING_FIX_SECONDS };
        }
        moved
    }

    /// Replaces the rest of journey `leg` with a route from a routing
    /// engine (real streets). Ignored when the bot is on another journey by
    /// now, or when the route leaves the region.
    pub fn follow(&mut self, town: &Town, leg: u32, route: &[Point]) {
        let Doing::Moving { path, walked_m, .. } = &mut self.doing else { return };
        if leg != self.leg || route.len() < 2 || !route.iter().all(|point| town.region.contains(*point)) {
            return;
        }
        *path = std::iter::once(self.at).chain(route.iter().copied()).collect();
        *walked_m = 0.0;
    }

    fn fix(&mut self) -> Fix {
        let jitter =
            Point { east: self.rng.between(-JITTER_M, JITTER_M), north: self.rng.between(-JITTER_M, JITTER_M) };
        let at = Point { east: self.at.east + jitter.east, north: self.at.north + jitter.north };
        let (speed_mps, heading) = match &self.doing {
            Doing::Staying { .. } => (0.0, None),
            Doing::Moving { path, walked_m, .. } => (self.speed_mps, along(path, *walked_m).1),
        };
        Fix { at, speed_mps, heading, battery_percent: self.battery.round() as i32 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::town::{LatLng, Region};

    fn town() -> Town {
        Town::new(Region { center: LatLng { lat: 35.7, lng: 139.58 }, radius_m: 900.0 }, &Rng::new(11))
    }

    /// Runs a walker for `seconds` in steps of five and returns what it did, with the time.
    fn run(walker: &mut Walker, town: &Town, seconds: u32) -> Vec<(f64, Moved)> {
        (1..=seconds / 5)
            .flat_map(|step| {
                let now = f64::from(step * 5);
                walker.advance(town, now, 5.0).into_iter().map(move |moved| (now, moved))
            })
            .collect()
    }

    #[test]
    fn a_walker_goes_from_place_to_place_at_a_human_speed() {
        let town = town();
        for (seed, cycling) in [(1, false), (2, true), (3, false)] {
            let mut walker = Walker::new(Rng::new(seed), &town, cycling);
            let log = run(&mut walker, &town, 3 * 3600);
            let arrivals = log.iter().filter(|(_, moved)| matches!(moved, Moved::Arrived { .. })).count();
            assert!(arrivals >= 5, "seed {seed}: {arrivals} arrivals in three hours");
            let fixes: Vec<(f64, &Fix)> = log
                .iter()
                .filter_map(|(at, moved)| if let Moved::Fix(fix) = moved { Some((*at, fix)) } else { None })
                .collect();
            let limit = if cycling { 4.8 } else { 1.6 };
            for pair in fixes.windows(2) {
                let ((before, a), (after, b)) = (pair[0], pair[1]);
                assert!(town.region.contains(b.at) || b.at.east.hypot(b.at.north) < 900.0 + 2.0 * JITTER_M);
                assert!(
                    a.at.distance(b.at) <= limit * (after - before) + 4.0 * JITTER_M,
                    "seed {seed}: jumped {} m",
                    a.at.distance(b.at)
                );
                assert!((15..=100).contains(&b.battery_percent));
            }
            assert!(fixes.iter().any(|(_, fix)| fix.speed_mps > 0.0 && fix.heading.is_some()));
            assert!(fixes.iter().any(|(_, fix)| fix.speed_mps == 0.0 && fix.heading.is_none()));
        }
    }

    #[test]
    fn fixes_come_often_on_the_way_and_seldom_at_rest() {
        let town = town();
        let mut walker = Walker::new(Rng::new(4), &town, false);
        let log = run(&mut walker, &town, 3600);
        let fixes: Vec<(f64, f64)> = log
            .iter()
            .filter_map(|(at, moved)| if let Moved::Fix(fix) = moved { Some((*at, fix.speed_mps)) } else { None })
            .collect();
        for pair in fixes.windows(2) {
            let gap = pair[1].0 - pair[0].0;
            assert!(gap <= STAYING_FIX_SECONDS, "{gap}");
            if pair[0].1 > 0.0 && pair[1].1 > 0.0 {
                assert_eq!(gap, MOVING_FIX_SECONDS);
            }
        }
    }

    #[test]
    fn a_route_from_the_engine_replaces_the_rest_of_the_journey() {
        let town = town();
        let mut walker = Walker::new(Rng::new(5), &town, false);
        let (leg, to) = (1..60)
            .find_map(|step| {
                let moved = walker.advance(&town, f64::from(step * 5), 5.0);
                moved
                    .into_iter()
                    .find_map(|moved| if let Moved::Departed { leg, to, .. } = moved { Some((leg, to)) } else { None })
            })
            .expect("it leaves within five minutes");
        let detour = Point { east: walker.at.east + 30.0, north: walker.at.north + 30.0 };
        // Another journey's route, and one that leaves the region, change nothing.
        let before = walker.clone();
        walker.follow(&town, leg + 1, &[detour, to]);
        walker.follow(&town, leg, &[Point { east: 5000.0, north: 0.0 }, to]);
        assert_eq!(format!("{:?}", walker.doing), format!("{:?}", before.doing));
        walker.follow(&town, leg, &[detour, to]);
        let Doing::Moving { path, walked_m, .. } = &walker.doing else { panic!("still on its way") };
        assert_eq!((path.as_slice(), *walked_m), (&[walker.at, detour, to][..], 0.0));
    }
}
