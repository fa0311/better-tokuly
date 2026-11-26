//! Chats: every now and then a short conversation happens in a room, a
//! line every few seconds, the speakers taking turns.
use crate::{lines::DIALOGUES, rng::Rng};

/// Where a conversation happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Room {
    /// The group chat of a group of the cast, by index.
    Group(usize),
    /// The direct messages of two bots (lower index first).
    Direct(usize, usize),
}

/// One line somebody says.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub speaker: usize,
    pub room: Room,
    pub text: &'static str,
}

#[derive(Debug, Clone)]
struct Talking {
    lines: &'static [&'static str],
    said: usize,
    speakers: Vec<usize>,
}

/// One room and the people in it.
#[derive(Debug, Clone)]
pub struct Chatter {
    rng: Rng,
    room: Room,
    members: Vec<usize>,
    talking: Option<Talking>,
    /// When the next line is said, or the next conversation starts.
    next: f64,
}

impl Chatter {
    /// A room with at least two members whose first conversation starts
    /// within a few minutes.
    pub fn new(mut rng: Rng, room: Room, members: Vec<usize>) -> Self {
        debug_assert!(members.len() >= 2);
        let next = rng.between(15.0, 240.0);
        Self { rng, room, members, talking: None, next }
    }

    /// The line said by `now`, if one is due.
    pub fn advance(&mut self, now: f64) -> Option<Line> {
        if now < self.next {
            return None;
        }
        let talking = self.talking.get_or_insert_with(|| {
            let mut speakers = self.members.clone();
            self.rng.shuffle(&mut speakers);
            speakers.truncate(2 + self.rng.below(2));
            Talking { lines: self.rng.pick(DIALOGUES), said: 0, speakers }
        });
        let line = Line {
            speaker: talking.speakers[talking.said % talking.speakers.len()],
            room: self.room,
            text: talking.lines[talking.said],
        };
        talking.said += 1;
        if talking.said == talking.lines.len() {
            self.talking = None;
            // Groups chat more than pairs do.
            let quiet = if matches!(self.room, Room::Group(_)) { 240.0..900.0 } else { 600.0..2400.0 };
            self.next = now + self.rng.between(quiet.start, quiet.end);
        } else {
            self.next = now + self.rng.between(6.0, 25.0);
        }
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_holds_whole_conversations_between_its_members() {
        let mut chatter = Chatter::new(Rng::new(2), Room::Group(0), vec![3, 5, 8, 9]);
        let lines: Vec<(f64, Line)> = (0..7200)
            .filter_map(|second| chatter.advance(f64::from(second)).map(|line| (f64::from(second), line)))
            .collect();
        assert!(lines.len() >= 20, "{} lines in two hours", lines.len());
        assert!(lines.iter().all(|(_, line)| [3, 5, 8, 9].contains(&line.speaker) && line.room == Room::Group(0)));
        // The first conversation is one of the scripts, in order, and nobody answers themselves.
        let script = DIALOGUES.iter().find(|script| script[0] == lines[0].1.text).expect("starts a script");
        for (said, text) in script.iter().enumerate() {
            assert_eq!(lines[said].1.text, *text);
            assert!(said == 0 || lines[said].1.speaker != lines[said - 1].1.speaker);
            assert!(said == 0 || lines[said].0 - lines[said - 1].0 <= 26.0);
        }
        // Then the room is quiet for a while.
        assert!(lines[script.len()].0 - lines[script.len() - 1].0 >= 240.0);
    }
}
