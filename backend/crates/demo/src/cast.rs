//! Who is in the demo: the bots' names and profiles, and who knows whom.
use crate::{
    lines::{CLUBS, GRADES, GROUP_NAMES, LIKES, NAME_MARKS, NAMES},
    rng::Rng,
};

/// One bot as people see it.
#[derive(Debug, Clone, PartialEq)]
pub struct Persona {
    pub handle: String,
    pub display_name: String,
    /// Says first of all that this is a bot of the demo.
    pub bio: String,
    /// Gets around by bicycle instead of on foot.
    pub cycling: bool,
    /// Seed of the avatar's colours.
    pub colours: u64,
}

/// A circle of friends with a group chat.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: &'static str,
    /// Indexes into the cast; the first one creates the chat.
    pub members: Vec<usize>,
}

/// The cast of `count` bots for a seed.
pub fn cast(rng: &Rng, count: usize) -> Vec<Persona> {
    let mut rng = rng.fork("cast");
    let mut names: Vec<usize> = (0..NAMES.len()).collect();
    rng.shuffle(&mut names);
    (0..count)
        .map(|index| {
            let name = &NAMES[names[index % names.len()]];
            // Past the last name the same names come round again, numbered.
            let round = index / names.len();
            let number = if round == 0 { String::new() } else { (round + 1).to_string() };
            Persona {
                handle: format!("demo_{}{number}", name.romaji),
                display_name: format!("{}{number}{}", name.kana, rng.pick(NAME_MARKS)),
                bio: format!(
                    "デモ用のボットです🤖 {}・{}。{}がすき！",
                    rng.pick(GRADES),
                    rng.pick(CLUBS),
                    rng.pick(LIKES)
                ),
                cycling: rng.chance(0.3),
                colours: rng.draw(),
            }
        })
        .collect()
}

/// The cast split into groups of four or so; nobody is left alone.
pub fn groups(count: usize) -> Vec<Group> {
    const SIZE: usize = 4;
    let mut groups: Vec<Group> = (0..count)
        .step_by(SIZE)
        .enumerate()
        .map(|(index, first)| Group {
            name: GROUP_NAMES[index % GROUP_NAMES.len()],
            members: (first..count.min(first + SIZE)).collect(),
        })
        .collect();
    if groups.len() > 1 && groups.last().is_some_and(|last| last.members.len() == 1) {
        let alone = groups.pop().expect("checked").members;
        groups.last_mut().expect("more than one").members.extend(alone);
    }
    groups
}

/// Who is friends with whom, as pairs `(lower, higher)` in order: everyone
/// within a group, and one friend in another group each.
pub fn friendships(rng: &Rng, groups: &[Group]) -> Vec<(usize, usize)> {
    let mut rng = rng.fork("friendships");
    let mut pairs = Vec::new();
    for group in groups {
        for (position, &a) in group.members.iter().enumerate() {
            pairs.extend(group.members[position + 1..].iter().map(|&b| (a, b)));
        }
    }
    for (index, group) in groups.iter().enumerate() {
        let others: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .flat_map(|(_, other)| other.members.iter().copied())
            .collect();
        if others.is_empty() {
            continue;
        }
        for &member in &group.members {
            let friend = *rng.pick(&others);
            pairs.push((member.min(friend), member.max(friend)));
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn a_seed_names_its_cast() {
        let rng = Rng::new(42);
        assert_eq!(cast(&rng, 12), cast(&rng, 12));
        assert_ne!(cast(&rng, 12), cast(&Rng::new(43), 12));
        // More bots add to the cast; those already there stay who they are.
        assert_eq!(
            cast(&rng, 12)[..5].iter().map(|p| &p.handle).collect::<Vec<_>>(),
            cast(&rng, 5).iter().map(|p| &p.handle).collect::<Vec<_>>()
        );
    }

    #[test]
    fn everyone_is_marked_as_a_demo_bot_and_fits_the_account_rules() {
        let cast = cast(&Rng::new(1), 60);
        let handles: HashSet<&str> = cast.iter().map(|p| p.handle.as_str()).collect();
        assert_eq!(handles.len(), cast.len(), "handles are unique");
        for persona in &cast {
            assert!(persona.handle.starts_with("demo_"));
            assert!(
                persona.handle.len() <= 20
                    && persona.handle.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            );
            assert!(persona.bio.starts_with("デモ用のボットです🤖"));
            assert!(persona.bio.chars().count() <= 160);
            assert!((1..=30).contains(&persona.display_name.chars().count()));
        }
    }

    #[test]
    fn groups_cover_everyone_once_and_leave_nobody_alone() {
        for count in 2..40 {
            let groups = groups(count);
            let mut members: Vec<usize> = groups.iter().flat_map(|g| g.members.iter().copied()).collect();
            members.sort_unstable();
            assert_eq!(members, (0..count).collect::<Vec<_>>());
            assert!(groups.iter().all(|g| g.members.len() >= 2), "{count}");
        }
        assert_eq!(groups(1).len(), 1);
        assert!(groups(0).is_empty());
    }

    #[test]
    fn friends_are_the_group_and_someone_outside_it() {
        let groups = groups(12);
        let pairs = friendships(&Rng::new(3), &groups);
        assert!(pairs.iter().all(|(a, b)| a < b));
        for group in &groups {
            assert!(pairs.contains(&(group.members[0], group.members[1])));
            for &member in &group.members {
                assert!(pairs.iter().any(|&(a, b)| (a == member && !group.members.contains(&b))
                    || (b == member && !group.members.contains(&a))));
            }
        }
        assert_eq!(friendships(&Rng::new(3), &groups[..1]).len(), 6);
    }
}
