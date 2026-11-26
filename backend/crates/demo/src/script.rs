//! The demo as a script: who does what, when — decided by the seed alone.
//!
//! [`Script::advance`] moves the simulated clock and returns what happened
//! in that time as [`Step`]s. It talks to nobody: the runner turns the
//! steps into API calls. Run twice with the same seed and episode, it tells
//! the same story (the tests hold it to that).
use crate::{
    cast::{self, Group, Persona},
    lines::SPOTS,
    posts::{Post, Poster},
    rng::Rng,
    talk::{Chatter, Line, Room},
    town::{LatLng, Region, Town},
    walk::{Fix, Moved, Walker},
};

/// Something one bot does.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Index into the cast.
    pub bot: usize,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Reports where the bot is.
    Locate {
        at: LatLng,
        fix: Fix,
        cycling: bool,
    },
    /// The bot set off on journey `leg`; a routing engine may know the way.
    Depart {
        leg: u32,
        from: LatLng,
        to: LatLng,
        cycling: bool,
    },
    /// Shows a status on the profile and the map.
    Status {
        emoji: &'static str,
        text: &'static str,
    },
    Post {
        post: Post,
        place: Option<Place>,
    },
    Say(Line),
}

/// The tag of a note written somewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: &'static str,
    pub at: LatLng,
}

pub struct Script {
    town: Town,
    cast: Vec<Persona>,
    groups: Vec<Group>,
    friendships: Vec<(usize, usize)>,
    walkers: Vec<Walker>,
    posters: Vec<Poster>,
    chatters: Vec<Chatter>,
    /// Seconds since the demo started.
    now: f64,
}

impl Script {
    /// The script of `episode` of the demo with this seed. The seed decides
    /// what stays: the cast, who is friends with whom, the town. The episode
    /// decides what happens: the walks, the posts, the conversations. A
    /// simulator that starts again plays a new episode with the same people
    /// instead of posting the same notes twice.
    pub fn new(seed: u64, episode: u64, bots: usize, region: Region) -> Self {
        let lasting = Rng::new(seed);
        let town = Town::new(region, &lasting);
        let cast = cast::cast(&lasting, bots);
        let groups = cast::groups(bots);
        let friendships = cast::friendships(&lasting, &groups);
        let rng = Rng::new(lasting.fork(&format!("episode {episode}")).draw());
        let friends_of = |bot: usize| -> Vec<usize> {
            friendships
                .iter()
                .filter_map(|&(a, b)| {
                    if a == bot {
                        Some(b)
                    } else if b == bot {
                        Some(a)
                    } else {
                        None
                    }
                })
                .collect()
        };
        let walkers = cast
            .iter()
            .enumerate()
            .map(|(bot, persona)| Walker::new(rng.fork(&format!("walk {bot}")), &town, persona.cycling))
            .collect();
        let posters = (0..bots).map(|bot| Poster::new(rng.fork(&format!("posts {bot}")), friends_of(bot))).collect();
        // A room per group, and direct messages between half of the friends.
        let mut pick = rng.fork("rooms");
        let rooms = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.members.len() >= 2)
            .map(|(index, group)| (Room::Group(index), group.members.clone()))
            .chain(friendships.iter().filter(|_| pick.chance(0.5)).map(|&(a, b)| (Room::Direct(a, b), vec![a, b])))
            .collect::<Vec<_>>();
        let chatters = rooms
            .into_iter()
            .map(|(room, members)| Chatter::new(rng.fork(&format!("talk {room:?}")), room, members))
            .collect();
        Self { town, cast, groups, friendships, walkers, posters, chatters, now: 0.0 }
    }

    pub fn cast(&self) -> &[Persona] {
        &self.cast
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// Pairs of bots that are friends, lower index first.
    pub fn friendships(&self) -> &[(usize, usize)] {
        &self.friendships
    }

    pub fn region(&self) -> Region {
        self.town.region
    }

    /// Lets `seconds` pass and returns what the bots did meanwhile, in
    /// order. Call it in small steps (a few seconds): a bot does each kind
    /// of thing at most once per call.
    pub fn advance(&mut self, seconds: f64) -> Vec<Step> {
        self.now += seconds;
        let (now, region) = (self.now, self.town.region);
        let mut steps = Vec::new();
        for (bot, walker) in self.walkers.iter_mut().enumerate() {
            let cycling = self.cast[bot].cycling;
            for moved in walker.advance(&self.town, now, seconds) {
                let action = match moved {
                    Moved::Fix(fix) => Action::Locate { at: region.locate(fix.at), fix, cycling },
                    Moved::Departed { leg, from, to } => {
                        Action::Depart { leg, from: region.locate(from), to: region.locate(to), cycling }
                    }
                    Moved::Arrived { spot } => Action::Status { emoji: SPOTS[spot].emoji, text: SPOTS[spot].status },
                };
                steps.push(Step { bot, action });
            }
            let spot = walker.staying_at();
            for post in self.posters[bot].advance(now, spot) {
                let place = match post {
                    Post::Note { spot: Some(spot), .. } => Some(Place {
                        name: SPOTS[spot].name,
                        at: region.locate(self.town.point(self.town.spot_corner(spot))),
                    }),
                    _ => None,
                };
                steps.push(Step { bot, action: Action::Post { post, place } });
            }
        }
        steps.extend(
            self.chatters
                .iter_mut()
                .filter_map(|chatter| chatter.advance(now))
                .map(|line| Step { bot: line.speaker, action: Action::Say(line) }),
        );
        steps
    }

    /// Puts the rest of bot's journey `leg` on a route a routing engine
    /// gave (see `Walker::follow`).
    pub fn follow(&mut self, bot: usize, leg: u32, route: &[LatLng]) {
        let route: Vec<_> = route.iter().map(|at| self.town.region.place(*at)).collect();
        self.walkers[bot].follow(&self.town, leg, &route);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const REGION: Region = Region { center: LatLng { lat: 35.703, lng: 139.5795 }, radius_m: 900.0 };

    fn hour(seed: u64, bots: usize) -> Vec<Step> {
        episode(seed, 0, bots)
    }

    fn episode(seed: u64, episode: u64, bots: usize) -> Vec<Step> {
        let mut script = Script::new(seed, episode, bots, REGION);
        (0..720).flat_map(|_| script.advance(5.0)).collect()
    }

    #[test]
    fn the_same_seed_tells_the_same_story() {
        let story = hour(2026, 12);
        assert_eq!(story, hour(2026, 12));
        assert_ne!(story, hour(2027, 12));
    }

    #[test]
    fn another_episode_has_the_same_people_doing_other_things() {
        let (first, second) = (Script::new(2026, 0, 12, REGION), Script::new(2026, 1, 12, REGION));
        assert_eq!(
            (first.cast(), first.groups(), first.friendships()),
            (second.cast(), second.groups(), second.friendships())
        );
        assert_eq!(first.town.spot_corner(0), second.town.spot_corner(0));
        assert_eq!(episode(2026, 1, 12), episode(2026, 1, 12));
        assert_ne!(episode(2026, 0, 12), episode(2026, 1, 12));
    }

    #[test]
    fn the_first_hour_shows_everything_the_app_does() {
        let story = hour(2026, 12);
        let bots_that = |wanted: fn(&Action) -> bool| {
            story.iter().filter(|step| wanted(&step.action)).map(|step| step.bot).collect::<HashSet<_>>()
        };
        assert_eq!(bots_that(|action| matches!(action, Action::Locate { .. })).len(), 12, "everyone is on the map");
        assert_eq!(bots_that(|action| matches!(action, Action::Depart { .. })).len(), 12, "everyone goes somewhere");
        assert_eq!(
            bots_that(|action| matches!(action, Action::Post { post: Post::Note { .. }, .. })).len(),
            12,
            "everyone writes a note"
        );
        assert!(!bots_that(|action| matches!(action, Action::Post { post: Post::Story { .. }, .. })).is_empty());
        assert!(!bots_that(|action| matches!(action, Action::Post { post: Post::Short { .. }, .. })).is_empty());
        assert!(!bots_that(|action| matches!(action, Action::Post { post: Post::Like { .. }, .. })).is_empty());
        assert!(!bots_that(|action| matches!(action, Action::Status { .. })).is_empty());
        assert!(bots_that(|action| matches!(action, Action::Say(Line { room: Room::Group(_), .. }))).len() >= 6);
        assert!(!bots_that(|action| matches!(action, Action::Say(Line { room: Room::Direct(..), .. }))).is_empty());
    }

    #[test]
    fn nobody_leaves_the_region_or_talks_to_strangers() {
        let mut script = Script::new(7, 0, 9, REGION);
        let friends: HashSet<(usize, usize)> = script.friendships().iter().copied().collect();
        let groups = script.groups().to_vec();
        for _ in 0..1440 {
            for step in script.advance(5.0) {
                match step.action {
                    // A fix may be a few meters off, like a phone's.
                    Action::Locate { at, .. } => {
                        assert!(REGION.place(at).distance(REGION.place(REGION.center)) <= REGION.radius_m + 5.0)
                    }
                    Action::Depart { from, to, .. } => {
                        assert!(REGION.contains(REGION.place(from)) && REGION.contains(REGION.place(to)))
                    }
                    Action::Say(Line { speaker, room: Room::Group(group), .. }) => {
                        assert!(groups[group].members.contains(&speaker))
                    }
                    Action::Say(Line { speaker, room: Room::Direct(a, b), .. }) => {
                        assert!(friends.contains(&(a, b)) && [a, b].contains(&speaker))
                    }
                    Action::Post {
                        post: Post::Like { author } | Post::Reply { author, .. } | Post::ShortReaction { author, .. },
                        ..
                    } => {
                        assert!(friends.contains(&(author.min(step.bot), author.max(step.bot))))
                    }
                    Action::Post { post: Post::StoryReaction { owner, .. }, .. } => {
                        assert!(friends.contains(&(owner.min(step.bot), owner.max(step.bot))))
                    }
                    Action::Post { place: Some(place), .. } => {
                        assert!(REGION.contains(REGION.place(place.at)) && !place.name.is_empty())
                    }
                    Action::Post { .. } | Action::Status { .. } => {}
                }
            }
        }
    }

    #[test]
    fn a_demo_of_one_or_two_bots_works() {
        for bots in [1, 2] {
            let story = hour(1, bots);
            assert!(story.iter().any(|step| matches!(step.action, Action::Locate { .. })));
            assert_eq!(story.iter().any(|step| matches!(step.action, Action::Say(_))), bots == 2);
        }
        assert!(hour(1, 0).is_empty());
    }
}
