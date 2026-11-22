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
| Blocking, muting and banning are not ways to find a sender out. | Nothing that already arrived disappears when the recipient blocks a sender (here or in the social graph) or mutes a friend. Only *signed* waiting items are withdrawn by a social block. A banned account's items do disappear (only moderators ban). |
| Moderators can always see the sender. | `AskModerationService.Inspect*` and, for takedowns, the subject resolver. Every lookup is recorded in `ask_inspections`. |

An image can only be attached to a **signed** question: the media API shows an
upload's owner to any signed-in caller (`Asset.owner_id` in the public view),
so an image would give an anonymous sender away.

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
| `ask.question.stored` | `{ id }` | tell the box owner (notification + realtime), if the question reached the inbox |
| `ask.response.stored` | `{ id }` | tell the prompt owner, if the response reached them |
| `ask.flame.stored` | `{ id }` | tell whoever was picked, if the flame reached them |

Requests to the generic domains (`tokuly_core::outbound`):

| Topic | What |
|---|---|
| `search.changed` | Kind `answer`, one document per **public** answer: `id` = the question's ID, `title` = the question, `text` = the answer, `owner_id` = the box owner, `path` = `/ask/q/<id>`, `extra = { anonymous, comment_count }`. Removed when the answer stops being public, is deleted or taken down, when the owner's account is deleted, and while the asker or the owner is banned. |
| `activity.recorded` | Kind `ask.answered` when an answer first appears on the Q&A page: audience `public` or `friends` like the answer, subject `SUBJECT_TYPE_QUESTION`, `data = { question }` (an excerpt). Private answers record nothing. |
| `notify.requested` | See "Notification kinds". |

## Events consumed

Durable consumer `ask` (`Ask::handle_event`). Every handler is safe to repeat.

| Topic | Reaction |
|---|---|
| `ask.question.stored`, `ask.response.stored`, `ask.flame.stored` | Delivery (above). Each item is claimed once (`delivered_at`). |
| `ask.answer.published` | With `first: true`: notifies the asker (`ask.answered`) and pushes `question_answered` to them. |
| `ask.prompt.opened` | Notifies the owner's friends (`ask.prompt`), except friends who muted the owner. |
| `identity.user.deleted` | Their box with its answers, their prompts with all responses, their rounds, settings, sender blocks and received flames are deleted (search documents removed, uploads released). What they sent stays with its recipient: the sender is cleared, signed items become anonymous, and a signed question loses its image. Sender blocks made *of* them stay, pointing at nobody. |
| `identity.user.banned` / `identity.user.unbanned` | Removes / restores the search documents of public answers in their box or to their questions. Lists need nothing: every recipient-side query leaves out what a banned account sent, for as long as the ban lasts. |
| `social.user.blocked` | Signed, still waiting questions and signed responses between the two are withdrawn (both directions); what this domain remembers of their friendship is forgotten. Anonymous items are left alone on purpose (see above). The box itself is `BOX_NOT_FOUND` for both from then on. |
| `social.friend.accepted` / `social.friend.removed` / `social.friend.prefs_changed` | Keeps `ask_friend_links`: since when two people are friends (for the "friends for N months" hint) and who muted whom (muted friends are not offered in the muter's polls; the muter is not told about that friend's prompts). The social domain's public API offers neither. |
| `admin.content.removed` | `subject_type = SUBJECT_TYPE_QUESTION`: the question, prompt or prompt response with that ID is removed for everyone, its sender included (`removed_by = 3`). Unknown or already removed IDs are done. |
| `media.asset.deleted` | The image is taken out of the question or answer that showed it. An answer that was only that image goes with it (the question is unanswered again). |

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
| Unpublished question (waiting, archived, filtered, or answered privately) — its recipient | yes | nil (nobody): reportable, tells nothing | no |
| Privately answered question — its asker | yes | the box owner | no (an interaction would name the asker) |
| Unpublished question — staff who are not a party to it | yes, also after its recipient deleted it | **the sender**; recorded in `ask_inspections` (once per staff member, item and hour) | no |
| Unpublished question — anyone else, and the asker of an unanswered one | no | | |
| Prompt — owner, friends, staff | yes | the prompt owner | no |
| Response — prompt owner | yes | nil, or the prompt owner once published | no |
| Response — friends (published only) / staff | yes | the prompt owner / the sender while unpublished (recorded) | no |
| Anything taken down by moderation | no | | |

A moderator who is the recipient or the sender of an item is treated as that,
not as staff. `counter_changed` keeps `like_count` and `comment_count` on the
question.

**Consequence for the admin domain:** a report filed by the recipient of an
anonymous question arrives with `subject_owner_id: null`. The case can be
resolved with REMOVE_CONTENT as it is; to warn or suspend the account that
wrote it, a moderator looks the sender up with
`AskModerationService.InspectQuestion` and sanctions that account directly.

## Realtime pushes

`ServerFrame.ask` (`AskEvent`), always user-scoped, never naming an anonymous
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

A hint is stored before the charge and marked paid after it, so a purchase
interrupted in between is completed by the next attempt without a second
charge. A round's reward that could not be paid when the round ended is paid
by the next `GetRound`.

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
| purge (`Ask::purge`) | every 10 minutes | Erases questions that were only kept for moderators (deleted, withdrawn, taken down, dropped) once older than the retention, prompts deleted longer ago than the retention, and rounds completed more than 30 days ago. |
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
hint purchases 30/min.

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
| `TOKULY_ASK_RETENTION_DAYS` | `180` | Days a removed or dropped question is kept for moderators; unanswered questions leave the sender's list after the same time. |

## Schema notes

- `ask_questions` keeps removed questions (`removed_by`: 1 owner, 2 sender,
  3 moderation) until the purge, so a report filed before can still be acted
  on. `dropped` marks what a blocked sender sent.
- `ask_poll_exposure` counts how often each friend was shown to a voter; a
  poll shows the four shown least. A friend gets their count the first time
  they are eligible, starting at the lowest count among the others.
- `ask_flame_hints` rows without `entry_id` are purchases in flight.
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
