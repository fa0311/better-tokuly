//! The realtime event messages are listed in three places: at `Event` in
//! `realtime.proto` (the source), in `src/events.rs` (which messages may be
//! published) and in the TypeScript client's table (checked by its own
//! test against the same list). This keeps the Rust list equal to the proto
//! list, and both to messages that exist.
use std::{collections::BTreeSet, fs, path::Path};

fn repo() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The full names listed in the comment of `Event`: lines of the form
/// `//   tokuly.<package>.v1.<Name>   what it is about`.
fn listed_in_proto() -> BTreeSet<String> {
    let proto = fs::read_to_string(repo().join("proto/tokuly/realtime/v1/realtime.proto")).unwrap();
    proto
        .lines()
        .filter_map(|line| line.strip_prefix("//   tokuly."))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(|name| format!("tokuly.{name}"))
        .collect()
}

/// The full names `src/events.rs` implements `RealtimeEvent` for: lines of
/// the form `"<feature>" => <package>::<Name>,`.
fn implemented_in_rust() -> BTreeSet<String> {
    let source = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/events.rs")).unwrap();
    source
        .lines()
        .filter_map(|line| {
            let (feature, path) = line.trim().strip_suffix(',')?.split_once(" => ")?;
            let (package, name) = path.split_once("::")?;
            assert_eq!(feature.trim_matches('"'), package, "an event is gated by the feature of its own package");
            Some(format!("tokuly.{package}.v1.{name}"))
        })
        .collect()
}

#[test]
fn the_publishable_events_are_the_ones_the_protocol_lists() {
    let listed = listed_in_proto();
    assert!(!listed.is_empty(), "the list at `Event` in realtime.proto was not found");
    assert_eq!(implemented_in_rust(), listed);
}

#[test]
fn every_listed_event_is_a_message_of_its_package() {
    for full_name in listed_in_proto() {
        let mut parts = full_name.split('.');
        let (package, name) = (parts.nth(1).unwrap(), parts.nth(1).unwrap());
        let file = repo().join(format!("proto/tokuly/{package}/v1/{package}.proto"));
        let proto = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        assert!(proto.contains(&format!("\nmessage {name} {{")), "{full_name} is not a message in {}", file.display());
    }
}
