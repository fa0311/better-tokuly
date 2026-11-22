# ask — events, jobs and configuration

Crate `tokuly-ask`, migration `1800_ask.sql`, API `tokuly.ask.v1`
(`QuestionBoxService`, `PromptService`, `ComplimentService`,
`AskModerationService`, realtime `AskEvent`). Depends on `identity`, `social`,
`media`, `stamp` and `wallet` (the ledger functions only).

Question boxes (質問箱), prompts (お題) and compliment polls (ほめ投票).

## Anonymity: what holds, and where it is decided

Three kinds of things are sent anonymously: questions, prompt responses and
compliment votes ("flames").

| Rule | How |
|---|---|
| The sender is stored and never returned to the recipient, unless the sender signed. | The row types the recipient's queries use select the sender only for signed items (`CASE WHEN signed THEN sender_id END`) and compare it with the viewer inside the database. Flame rows have no sender column at all. |
| The recipient can block "whoever sent this" without learning who it is. | `BlockSender` copies the sender from the item's row into `ask_sender_blocks` inside the database. One row per blocked item, so the list of blocks does not show that two items came from one person; a block survives the sender's account. |
| A blocked sender cannot tell. | Their request runs the same statements as anyone's: the same limits are counted (attempts, before the block is looked up), the same row and the same `ask.*.stored` event are written, the same response is returned. The row only carries `dropped = true`, which the delivery worker and every recipient-side query skip. Dropped votes advance the round and earn the same coins. |
| The sender learns nothing else either. | They see "waiting" or "answered". Read, archived, filtered, deleted and dropped all look like "waiting"; unanswered questions leave the sender's list after `TOKULY_ASK_RETENTION_DAYS`, whatever became of them, so the purge is not visible. |
| Blocking and muting are not ways to find a sender out. | Nothing that already arrived disappears when the recipient blocks a sender (here or in the social graph) or mutes a friend. Only *signed* waiting items are withdrawn by a social block. |
| Nothing anonymous follows the sender's account. | See "What a sender's account does to what they sent". |
| Anonymity needs a crowd. | Where only the recipient's friends can send (a friends-only box, a prompt, a compliment poll), the recipient needs **three friends** before anything anonymous is accepted for them: `AUDIENCE_TOO_SMALL` for a question or response (a signed one is accepted; `QuestionBox.anonymous_available` and `Prompt.anonymous_available` say so beforehand), and such a friend is not seated in polls. The number of friends is on every profile, so the refusal tells the sender nothing. |
| What reached the recipient can be blocked and reported, whatever became of it. | `BlockSender` and the subject resolver work on anything that was delivered: also after the recipient deleted it or the sender withdrew it. |
| Moderators can see the sender, except of what was sent to or by themselves. | `AskModerationService.Inspect*` (`CONFLICT_OF_INTEREST` for a party to the item) and, for takedowns, the subject resolver. Every lookup is recorded in `ask_inspections`. |

### What a sender's account does to what they sent

If an anonymous item appeared, disappeared or changed when something happened
to an account, whoever noticed both would know who sent it. So the recipient's
view is fixed at two moments and never follows the sender afterwards:

- **Delivery.** An item is in nobody's list until the delivery worker has
  dealt with it (`delivered_at`); from then on it stays. The worker is the
  one place that looks at the sender's account: an item whose sender is
  suspended at that moment is stopped (stored as dropped, like one from a
  blocked sender).
- **The vote.** Whether a flame sells hints (the voter's `allow_hints`) and
  what they say are worked out when the vote is cast and stored on the flame
  (`ask_flames.hints`). `BuyHint` reads the flame and nothing else: the price
  list and its errors are the same whatever the voter did since.

| The sender's account is… | Anonymous items they sent | Signed items they sent |
|---|---|---|
| suspended | unchanged everywhere: inbox, counts, Q&A page, search index, flames, ranking, hints, export | hidden while the suspension lasts (their name is on them), back afterwards |
| deleted | unchanged, pictures included (the item keeps holding the upload, so the media domain keeps it) | stay, without the name: they become anonymous |
| blocked by the recipient (social graph) | unchanged | withdrawn if still waiting |

A single item that has to go is taken down by moderation
(`admin.content.removed`).

### Pictures on anonymous items

A question or a prompt response can carry a picture, signed or not, when the
recipient's `allow_images` is on. What was checked: the media pipeline
re-encodes every image to WebP (EXIF, GPS and the other metadata are gone, the
original is never served), the public view of an asset has neither its owner
nor its file name, and its URL is made of the asset's ID and a hash of the
pixels. What remains is the upload time (inside the asset ID) and the picture
itself. The one thing that would link a picture to a person is the same upload
appearing twice, so a picture sent here must be an upload nothing else holds
(`IMAGE_ALREADY_USED`). Not enforceable from this domain: the sender attaching
the same upload somewhere else *afterwards*; clients upload the file once per
question and do not offer it again.

## Events emitted

Payload structs live in `tokuly_ask::events`. All are written in the
transaction of the change.

| Topic | Payload | When |
|---|---|---|
| `ask.answer.published` | `{ question_id, owner_id, audience, first }` | The owner of a box answered a question (`first: true`), or an edit changed the answer's audience (`first: false`). `audience` is `tokuly.ask.v1.AnswerAudience` as its number (1 public, 2 friends, 3 private: only the asker and the owner). Actor: the owner. Says nothing about who asked. |
| `ask.answer.deleted` | `{ question_id, owner_id }` | An answer went away: deleted by its owner, with its question, by a takedown, or because its only content was an upload that was deleted. Actor: the owner, or nobody. |
| `ask.prompt.opened` | `{ prompt_id, owner_id, closes_at }` | A prompt was posted to the owner's friends. Actor: the owner. |
| `ask.round.completed` | `{ user_id, round_id, votes, skipped, coins }` | A user finished a round of compliment polls. `coins` is what the round pays. Actor: the user. Says nothing about who was picked. |

Delivery events — this domain's own queue. **Do not consume them**: they are
written for dropped and held items too, carry no actor and nothing but an ID.

| Topic | Payload | Consumed by this domain to |
|---|---|---|
| `ask.question.stored` | `{ id }` | deliver the question, and tell the box owner (notification + realtime) if it reached the inbox |
| `ask.response.stored` | `{ id }` | deliver the response, and tell the prompt owner if it reached them |
| `ask.flame.stored` | `{ id }` | deliver the flame, and tell whoever was picked if it reached them |

Requests to the generic domains (`tokuly_core::outbound`):

| Topic | What |
|---|---|
| `search.changed` | Kind `answer`, one document per **public** answer: `id` = the question's ID, `title` = the question, `text` = the answer, `owner_id` = the box owner, `path` = `/ask/q/<id>`, `extra = { anonymous, comment_count }`. Removed when the answer stops being public, is deleted or taken down, when the owner's account is deleted, and while the owner, or the asker of a *signed* question, is suspended. |
| `activity.recorded` | Kind `ask.answered` when an answer first appears on the Q&A page: audience `public` or `friends` like the answer, subject `SUBJECT_TYPE_QUESTION`, `data = { question }` (an excerpt). Private answers record nothing. |
| `notify.requested` | See "Notification kinds". |

## Events consumed

Durable consumer `ask` (`Ask::handle_event`). Every handler is safe to repeat.

| Topic | Reaction |
|---|---|
| `ask.question.stored`, `ask.response.stored`, `ask.flame.stored` | Delivery (above). Each item is dealt with once (`delivered_at`), also when the event arrives several times at once; until then its recipient does not have it. |
| `ask.answer.published` | With `first: true`: notifies the asker (`ask.answered`) and pushes `question_answered` to them. |
| `ask.prompt.opened` | Notifies the owner's friends (`ask.prompt`), except friends who muted the owner. |
| `identity.user.deleted` | Their box with its answers, their prompts with all responses, their rounds, settings, sender blocks and received flames are deleted (search documents removed, uploads released). What they sent stays with its recipient as it was, pictures included: the sender is cleared and signed items become anonymous. Sender blocks made *of* them stay, pointing at nobody; flames keep the hints they were sold with. |
| `identity.user.banned` / `identity.user.unbanned` | Removes / restores the search documents of public answers in their box and to questions they asked **under their name**. Lists need nothing: they leave out the signed items of a suspended account themselves. Anonymous items are not touched. |
| `social.user.blocked` | Signed, still waiting questions and signed responses between the two are withdrawn (both directions); what this domain remembers of their friendship is forgotten. Anonymous items are left alone on purpose (see above). The box itself is `BOX_NOT_FOUND` for both from then on. |
| `social.friend.accepted` / `social.friend.removed` / `social.friend.prefs_changed` | Keeps `ask_friend_links`: since when two people are friends (for the "friends for N months" hint) and who muted whom (muted friends are not offered in the muter's polls; the muter is not told about that friend's prompts). The social domain's public API offers neither. |
| `admin.content.removed` | `subject_type = SUBJECT_TYPE_QUESTION`: the question, prompt or prompt response with that ID is removed for everyone, its sender included (`removed_by = 3`). Unknown or already removed IDs are done. |
| `media.asset.deleted` | The image is taken out of the question, prompt response or answer that showed it. An answer that was only that image goes with it (the question is unanswered again). |

## Subjects

`SUBJECT_TYPE_QUESTION` resolves three things by ID: a question (with its
answer), a prompt, and a prompt response. There is no subject type for the
last two, and everything anonymous has to be reportable.

Who the generic domains are told the owner is depends on the viewer, because
engagement hides a subject from anyone in a block with its owner — a rule the
recipient of an anonymous question could otherwise use to find its sender:

| Subject, as seen by | Resolves? | `owner_id` | Likes / comments |
|---|---|---|---|
| Question with a public or friends answer — anyone the audience includes, the asker, the owner, staff | yes | the box owner | yes |
| Unpublished question (waiting, archived, filtered, answered privately; also one the recipient deleted or the sender withdrew) — its recipient | yes, if it was ever delivered | nil (nobody): reportable, tells nothing | no |
| Privately answered question — its asker | yes | the box owner | no (an interaction would name the asker) |
| Unpublished question — staff who are not a party to it | yes, also after its recipient deleted it | **the sender**; recorded in `ask_inspections` (once per staff member, item and hour) | no |
| Unpublished question — anyone else, and the asker of an unanswered one | no | | |
| Prompt — owner, friends, staff | yes | the prompt owner | no |
| Response — prompt owner | yes, if it was ever delivered (also after deleting it) | nil, or the prompt owner once published | no |
| Response — friends (published only) / staff | yes | the prompt owner / the sender while unpublished (recorded) | no |
| Anything taken down by moderation | no | | |

A moderator who is the recipient or the sender of an item is treated as that,
not as staff, and `AskModerationService.Inspect*` refuses them
(`CONFLICT_OF_INTEREST`): a colleague handles the report. `counter_changed` keeps `like_count` and `comment_count` on the
question.

**Consequence for the admin domain:** a report filed by the recipient of an
anonymous question arrives with `subject_owner_id: null`. The case can be
resolved with REMOVE_CONTENT as it is; to warn or suspend the account that
wrote it, a moderator looks the sender up with
`AskModerationService.InspectQuestion` and sanctions that account directly.

## Realtime pushes

`AskEvent`, always user-scoped, never naming an anonymous
sender:

| Event | Sent to | When |
|---|---|---|
| `question_received` | box owner | A question reached the inbox (not for held or dropped ones). Carries the question as the owner sees it and the inbox counts. |
| `question_answered` | asker | Their question was answered. |
| `flame_received` | the picked friend | A flame reached them. |
| `prompt_response_received` | prompt owner | A response reached them. Carries the prompt with its counters. |

## Notification kinds

Notifications about anonymous items have **no actor** and no name, and no
per-friend mute is applied to them (it would be a way to find the sender).

| Kind | Recipient | Actor | Collapse key | Path |
|---|---|---|---|---|
| `ask.question` | box owner | the asker if signed, else none | `ask.question:<owner>` | `/ask/inbox` |
| `ask.answered` | asker | the box owner | `ask.answered:<question>` | `/ask/q/<question>` |
| `ask.prompt` | each friend of the owner who did not mute them | the owner | `ask.prompt:<owner>` | `/ask/prompts/<prompt>` |
| `ask.prompt_response` | prompt owner | the responder if signed, else none | `ask.prompt_response:<prompt>` | `/ask/prompts/<prompt>` |
| `ask.flame` | the picked friend | none | `ask.flame:<receiver>` | `/ask/flames` |

The text of an anonymous question is never quoted in a notification.

## Coins

Through `tokuly_wallet::ledger`; memos carry IDs of the user's own things only.

| Reason | Movement | Idempotency key | Memo |
|---|---|---|---|
| `ask.round` | grant: `round_reward_coins × votes / polls` for a completed round (nothing for a round that was only skipped) | `ask.round:<round id>` | `{ round_id }` |
| `ask.hint` | spend: the price of the next hint about a flame's sender | `ask.hint:<flame id>:<0 or 1>` | `{ flame_id, hint }` |

What a hint says is on the flame since the vote; a purchase charges under a
key made of the flame and the hint's position and stores the hint afterwards.
However often a purchase is repeated, and however many arrive at once, one
hint is paid for once; a purchase interrupted after the charge is completed by
the next attempt without a second one. A round's reward that could not be paid
when the round ended is paid by the next `GetRound`, whether or not the
cooldown is over by then.

## Profile, export

- `Ask::new` registers a `tokuly_identity::ProfileExtension`: profiles of users
  whose box is open get a `ProfileCard` of kind `ask.box` (title 質問箱 /
  Question box, the owner's prompt line, path `/u/<handle>/ask`,
  `data = { audience: "friends" | "signed_in", can_ask, answer_count }`, order 40).
- A `DataExporter` named `ask`: settings, questions received (without the
  sender unless signed) and sent ("answered" or not, like the list), sender
  blocks, prompts with responses, responses sent, flames received with bought
  hints, votes cast, rounds.

## Background work

No job-queue jobs. `Ask::spawn_workers` starts:

| Worker | Schedule | What it does |
|---|---|---|
| event consumer `ask` | on events | See "Events consumed". |
| purge (`Ask::purge`) | every 10 minutes | Erases what was only kept for moderators once the retention has passed **since it was removed** (dropped items: since they were sent): questions and prompt responses that were deleted, withdrawn, taken down or dropped, and deleted prompts with what is left on them. Releases their uploads. Also erases rounds completed more than 30 days ago. |
| bank loader | once at start | Copies the built-in poll questions (`crates/ask/data/poll_questions.tsv`, 162 questions in Japanese and English) into `ask_poll_questions`. New ones arrive enabled, known ones get their current wording and keep the `enabled` staff left them with, removed ones are switched off. Also runs before the first poll of a process. |

## Roles

| Role | RPCs |
|---|---|
| `moderator` | `InspectQuestion`, `InspectPromptResponse`, `InspectFlame`, `ListPollQuestions` |
| `admin` | the above plus `AddPollQuestion`, `SetPollQuestionEnabled` |

## Valkey keys

| Key | Type, TTL | Purpose |
|---|---|---|
| `ask:inspected:<staff>:<item>` | marker, 1 h | The resolver already recorded this staff member's view of this item's sender. |

Rate limits (`rl:` keys): questions and prompt responses per sender
(`ask:send:`), per sender and recipient (`ask:pair:`) and per recipient
(`ask:box:`, reason `BOX_BUSY`) as configured below; answers 120/h per owner;
prompts 10/day per owner (and at most 3 open at once); sender blocks 120/h;
hint purchases 30/min; round requests (`GetRound`, `Vote`, `SkipPoll`,
`ShufflePoll` together) 120/min per user (`ask:round:`).

## Connections and locks

Everything one user does to their rounds is serialized by an advisory lock
(`ask:round:<user>`). A request reads what it needs first (who can be shown,
who would be seated, what a vote sells as hints) and takes the lock last;
while it holds the lock, or any transaction, it uses that transaction's
connection only. No code in the crate asks the pool for a connection while it
holds one: a pool's worth of requests doing that at once would wait for each
other.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_ASK_FILTER_WORDS` | a built-in list of insults, calls to self-harm and sexual solicitation in Japanese and English | Comma-separated words that hold a question or prompt response in the recipient's FILTERED folder, on top of each owner's muted words. Latin words match whole words, others as substrings (ignoring separators, width and case). Empty switches the server-wide list off. |
| `TOKULY_ASK_SENDS_PER_HOUR` | `30` | Questions and prompt responses one account may send per hour. |
| `TOKULY_ASK_PAIR_SENDS_PER_DAY` | `10` | … to the same person per day. |
| `TOKULY_ASK_BOX_RECEIVES_PER_HOUR` | `120` | Questions and prompt responses one person may receive per hour from everyone together (the brake on a pile-on). |
| `TOKULY_ASK_ROUND_POLLS` | `12` | Polls in a compliment round (1–50). |
| `TOKULY_ASK_ROUND_COOLDOWN_SECONDS` | `3600` | Seconds between finishing a round and the next one opening. |
| `TOKULY_ASK_ROUND_REWARD_COINS` | `12` | Coins for a round in which every poll was voted on. |
| `TOKULY_ASK_SHUFFLES_PER_POLL` | `3` | Times the four friends of one poll can be replaced. |
| `TOKULY_ASK_HINT_PRICES` | `30,60` | Price of the first and the second hint about a flame's sender. Empty sells none. |
| `TOKULY_ASK_RETENTION_DAYS` | `180` | Days a removed or dropped question or prompt response is kept for moderators, counted from its removal; unanswered questions leave the sender's list the same number of days after they were sent. |

## Schema notes

- `ask_questions` keeps removed questions (`removed_by`: 1 owner, 2 sender,
  3 moderation) until the purge, so a report filed before can still be acted
  on. `dropped` marks what never reached its recipient: sent by a sender
  the recipient blocked, or by an account that was suspended before the
  item was delivered. `delivered_at` is set once the delivery worker dealt
  with an item; recipient-side queries show delivered items only.
- `ask_poll_exposure` counts how often each friend was shown to a voter; a
  poll shows the four shown least. A friend gets their count the first time
  they are eligible, starting at the lowest count among the others.
- `ask_flames.hints` is what a flame sells, fixed when the vote was cast;
  `ask_flame_hints` has one row per hint that was paid for, with the buyer's
  idempotency keys that led to it.
- A poll seats only friends who themselves have three friends
  (`senders::MIN_ANONYMITY_SET`).
- `ask_inspections` is the record of every time staff were told a sender.

## Wiring

```rust
let ask = Ask::new(core.clone(), identity.clone(), cfg.ask)?;   // after Identity; registers resolver, profile card, exporter
router = ask.register(router);                                 // the four services
ask.spawn_workers();                                           // worker processes only
```

Migration `1800_ask.sql` needs `0001`, `0100` (it references `users` only);
the crate also reads the tables of `social`, `media`, `stamp` and `wallet`
through their public functions.
