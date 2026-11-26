//! What one bot posts: notes, stories, shorts, and its reactions to what
//! its friends posted.
use crate::{
    lines::{NOTES, REACTIONS, REPLIES, SHORT_CAPTIONS, SPOTS, STORY_CAPTIONS},
    rng::Rng,
};

/// Clips a short can be made of (see `media::CLIPS`).
pub const CLIP_COUNT: usize = 3;

/// Something a bot puts out.
#[derive(Debug, Clone, PartialEq)]
pub enum Post {
    /// A note; `spot` tags it with the place it was written at.
    Note {
        text: &'static str,
        spot: Option<usize>,
    },
    /// A story of one picture, painted from `colours`.
    Story {
        caption: &'static str,
        colours: u64,
    },
    Short {
        caption: &'static str,
        clip: usize,
    },
    /// The newest note of bot `author` gets a like.
    Like {
        author: usize,
    },
    /// ... or an answer.
    Reply {
        author: usize,
        text: &'static str,
    },
    /// The newest story of friend `owner` gets an emoji.
    StoryReaction {
        owner: usize,
        emoji: &'static str,
    },
    ShortReaction {
        author: usize,
        emoji: &'static str,
    },
}

#[derive(Debug, Clone)]
pub struct Poster {
    rng: Rng,
    /// The other bots this one is friends with.
    friends: Vec<usize>,
    next_note: f64,
    next_story: f64,
    next_short: f64,
    next_reaction: f64,
}

impl Poster {
    /// Everyone posts a note in the first minutes, so the timeline of a
    /// fresh demo is not empty; some open with a story or a short.
    pub fn new(mut rng: Rng, friends: Vec<usize>) -> Self {
        let next_note = rng.between(5.0, 180.0);
        let next_story = if rng.chance(0.4) { rng.between(20.0, 240.0) } else { rng.between(1800.0, 4.0 * 3600.0) };
        let next_short = if rng.chance(0.25) { rng.between(30.0, 300.0) } else { rng.between(3600.0, 6.0 * 3600.0) };
        let next_reaction = rng.between(60.0, 240.0);
        Self { rng, friends, next_note, next_story, next_short, next_reaction }
    }

    /// What the bot posts by `now`; `spot` is where it is staying, if anywhere.
    pub fn advance(&mut self, now: f64, spot: Option<usize>) -> Vec<Post> {
        let mut posts = Vec::new();
        if now >= self.next_note {
            self.next_note = now + self.rng.between(900.0, 2700.0);
            posts.push(match spot {
                Some(spot) if self.rng.chance(0.4) => {
                    Post::Note { text: self.rng.pick(SPOTS[spot].notes), spot: Some(spot) }
                }
                _ => Post::Note { text: self.rng.pick(NOTES), spot: None },
            });
        }
        if now >= self.next_story {
            self.next_story = now + self.rng.between(1.5 * 3600.0, 4.0 * 3600.0);
            posts.push(Post::Story { caption: self.rng.pick(STORY_CAPTIONS), colours: self.rng.draw() });
        }
        if now >= self.next_short {
            self.next_short = now + self.rng.between(2.0 * 3600.0, 6.0 * 3600.0);
            posts.push(Post::Short { caption: self.rng.pick(SHORT_CAPTIONS), clip: self.rng.below(CLIP_COUNT) });
        }
        if now >= self.next_reaction {
            self.next_reaction = now + self.rng.between(60.0, 240.0);
            if !self.friends.is_empty() {
                let friend = *self.rng.pick(&self.friends);
                posts.push(match self.rng.below(20) {
                    0..10 => Post::Like { author: friend },
                    10..13 => Post::Reply { author: friend, text: self.rng.pick(REPLIES) },
                    13..17 => Post::StoryReaction { owner: friend, emoji: self.rng.pick(REACTIONS) },
                    _ => Post::ShortReaction { author: friend, emoji: self.rng.pick(REACTIONS) },
                });
            }
        }
        posts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bot_posts_soon_and_keeps_a_believable_pace() {
        let mut poster = Poster::new(Rng::new(6), vec![1, 2]);
        let day: Vec<(u32, Post)> = (0..24 * 720)
            .flat_map(|step| poster.advance(f64::from(step * 5), Some(3)).into_iter().map(move |post| (step * 5, post)))
            .collect();
        let count = |wanted: fn(&Post) -> bool| day.iter().filter(|(_, post)| wanted(post)).count();
        assert!(matches!(day[0], (0..=180, Post::Note { .. })), "{:?}", day[0]);
        let notes = count(|post| matches!(post, Post::Note { .. }));
        assert!((30..=100).contains(&notes), "{notes} notes a day");
        assert!((5..=17).contains(&count(|post| matches!(post, Post::Story { .. }))));
        assert!((3..=13).contains(&count(|post| matches!(post, Post::Short { .. }))));
        for (_, post) in &day {
            match post {
                Post::Note { text, spot } => {
                    assert!(!text.is_empty() && spot.is_none_or(|spot| spot == 3 && SPOTS[3].notes.contains(text)))
                }
                Post::Short { clip, .. } => assert!(*clip < CLIP_COUNT),
                Post::Like { author } | Post::Reply { author, .. } | Post::ShortReaction { author, .. } => {
                    assert!([1, 2].contains(author))
                }
                Post::StoryReaction { owner, .. } => assert!([1, 2].contains(owner)),
                Post::Story { .. } => {}
            }
        }
    }

    #[test]
    fn a_bot_without_friends_reacts_to_nobody() {
        let mut poster = Poster::new(Rng::new(6), vec![]);
        let posts: Vec<Post> = (0..3600).flat_map(|second| poster.advance(f64::from(second), None)).collect();
        assert!(
            posts
                .iter()
                .all(|post| matches!(post, Post::Note { spot: None, .. } | Post::Story { .. } | Post::Short { .. }))
        );
    }
}
