# Before you open (or update) a backend pull request: check your own crate against this list

The first domain that went through independent review passed its 123 tests and CI and still came back
with four major findings. None was exotic. Every reviewer will look for the same things in your crate, so
find them first: go through the list with your code open, fix what you find, and add the test that would
have caught it. Say in your report which items led to a change.

1. **A database connection held while asking for another.** A handler that opens a transaction or takes an
   advisory lock and then runs a query through the pool (`&core.db`, or a helper of another domain that uses
   the pool) needs two connections at once. A few concurrent requests from ONE account then leave the lock
   holder without a connection, and every request of every user waits out the pool's acquire timeout.
   Inside a transaction, read on the transaction; or read before you take the lock. Search your crate for
   every `begin()` / `pg_advisory` and read what happens before the commit, including inside the functions
   you call. Then prove it: a test that fires as many concurrent calls as the test pool has connections at
   the RPCs that lock, from one account.
2. **No rate limit on something one account can repeat to hurt others or the server**: anything that
   notifies someone, sends a push, writes a row per call, fans out, or is expensive to compute.
3. **What someone sees changes with another person's account state, and that tells them something.** When
   an account is suspended, deleted, blocked or changes a setting, what disappears from, or changes in,
   other people's lists, counts, rankings, search results and exports? If the feature hides who did
   something (anonymous authors, secret voters, hidden members, ghost mode), nothing visible to the other
   side may move with that account's state. The same for error answers: two different errors, or an error
   before versus after a charge, are a way to ask a yes/no question about someone for free.
4. **An action that removes the handle the victim needs.** If the sender can withdraw, delete, edit or leave
   and the recipient then cannot block, mute or report what they already received, harassment is free.
   Blocking and reporting must work on anything that was delivered.
5. **Money and rewards settle exactly once, and are not lost.** A grant or charge that failed after the state
   change must be retried from every later entry point, not only inside some time window; decide what a
   paid-for thing contains at the moment of payment and store it, instead of recomputing it from live state.
6. **Two requests at once.** For every "check, then write" in your crate: what happens when two requests
   pass the check together? The database must enforce it (unique index, `FOR UPDATE`, a conditional
   `UPDATE … WHERE` whose row count you check). Write at least one test that really runs requests
   concurrently (`tokio::join!` / `futures::future::join_all`) for each place that depends on a lock,
   uniqueness or ordering.
7. **Staff powers used on oneself.** A moderator or admin RPC that reveals hidden information must refuse
   when the staff member is a party to the item.
8. **Small groups defeat anonymity and privacy.** "Anonymous among friends" with one friend is not
   anonymous; an aggregate over one person is that person's value. Give such features a minimum group size.
9. **Retention and purge clocks.** A purge that counts from creation instead of from removal deletes
   evidence the moment the owner removes something old; a report filed just before can no longer be acted on.
10. **Events are delivered at least once and not always in order.** Every consumer must give the same result
    when an event arrives twice, late, or after the row it refers to is gone.

Your own files only; local checks green before you push.
