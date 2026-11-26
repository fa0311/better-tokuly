//! The tests' Valkey database: logical database [`DATABASE`] instead of the
//! one in `TOKULY_VALKEY_URL`, which a dev server on the same machine uses.
//!
//! Keys of different test runs do not meet (they carry fresh IDs), but a
//! dev server's do: LiveKit posts its webhooks there, and the server's
//! claim on a venue's comparison (`spaces:compare:<room>`) would swallow
//! the comparison a test asked for. Valkey has no per-run namespace to
//! create and remove, so the runs share this one; every key in it expires
//! or is deleted by the test that wrote it.

/// Valkey's last logical database in its default configuration (16 of them).
const DATABASE: u8 = 15;

/// Points a `redis://` or `rediss://` URL at [`DATABASE`]; other URLs (a
/// Unix socket) are left alone.
pub(crate) fn for_tests(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://").filter(|(scheme, _)| matches!(*scheme, "redis" | "rediss")) else {
        return url.to_owned();
    };
    let (rest, query) = rest.split_once('?').map_or((rest, None), |(rest, query)| (rest, Some(query)));
    let authority = rest.split_once('/').map_or(rest, |(authority, _)| authority);
    match query {
        Some(query) => format!("{scheme}://{authority}/{DATABASE}?{query}"),
        None => format!("{scheme}://{authority}/{DATABASE}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_database_is_replaced_or_added() {
        assert_eq!(for_tests("redis://127.0.0.1:6379"), "redis://127.0.0.1:6379/15");
        assert_eq!(for_tests("redis://127.0.0.1:6379/"), "redis://127.0.0.1:6379/15");
        assert_eq!(for_tests("rediss://user:pw@valkey:6380/2?timeout=5"), "rediss://user:pw@valkey:6380/15?timeout=5");
        assert_eq!(for_tests("unix:///run/valkey.sock"), "unix:///run/valkey.sock");
    }
}
