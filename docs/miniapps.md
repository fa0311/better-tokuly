# Mini apps

A mini app is the web page of a developer platform application, opened
inside Tokuly from a card in a conversation. It runs in a sandboxed frame and
reaches Tokuly only through a message bridge.

| Part | Where |
|---|---|
| Manifest, review, grants | crate `tokuly-dev` (`mini_apps.rs`), migration `2401_dev_mini_apps.sql`, `MiniAppService` and `MiniAppReviewService` in `tokuly.dev.v1` |
| Bridge messages | `@tokuly/miniapp/protocol` (hosts and SDK) |
| SDK | `@tokuly/miniapp` |
| Host logic shared by the apps | `@tokuly/core/miniapps` |
| Web host | `apps/web/src/features/miniapps` (`/miniapps/<id>`, `/admin/mini-apps`), the manifest form on `/developers/apps/<id>` |
| Sample (割り勘), development only | `apps/miniapp-sample` |

## Review

```
SetMiniAppManifest ─▶ draft ─SubmitMiniApp─▶ in review ─ReviewMiniApp─▶ approved (draft copied to live)
       ▲                                         │                   └▶ rejected (note to the owner)
       └──────────── any edit: a new revision, back to draft ◀───────┘
```

- A manifest is a name (1-30 characters), an icon, a launch URL and the
  mini-app scopes it asks for. People open the approved copy (`live`); an
  edit makes a new draft revision and the approved copy keeps running until
  that revision is approved.
- A moderator decides on the revision they read (`REVIEW_OUTDATED` when it
  changed or was decided since; one decision wins when two are made at once)
  and never on their own application (`OWN_APP`). Rejecting needs a note.
  The owner is notified (`dev.mini_app_reviewed`).
- A mini app goes with its application: unusable while the application is
  disabled by moderation or its owner suspended, deleted with it. The owner
  can take it out of Tokuly (`DeleteMiniApp`) unless moderation disabled the
  application.

## Opening one

A card is a chat message whose text ends in `<web origin>/miniapps/<app id>[?p=<path>]`;
clients draw it as the mini app's card. Opening it loads
`/miniapps/<id>?p=<path>&c=<conversation>&o=<approved origin>` as a new page:

- The page's Content-Security-Policy adds `o` (a bare `https` origin; `http`
  on a loopback address outside production) to `frame-src`, on this page
  only. The page frames the mini app only after `GetMiniApp` confirmed that
  origin, and reloads itself with the right one when they differ, so a
  hand-made `o` allows a frame nothing ever loads.
- The frame shows the launch URL, or the card's path on the same origin.

## Security model

- **Sandbox.** `sandbox="allow-scripts allow-forms"`: an opaque origin
  (no cookies or storage of any site, no access to the page around it), no
  top navigation, pop-ups or dialogs. `allow=""`: no powerful features.
  `allow-same-origin` stays off; a mini app talks to its server with CORS
  without credentials.
- **Pinned origin.** `frame-src` holds the approved origin, so neither the
  mini app nor anything it links to can turn the frame into another site.
  Launch URLs are `https`, on a domain that is not Tokuly's or under it.
- **Who is talking.** The host answers a message only when it comes from the
  frame's window with the opaque origin `"null"`; anything else (the page
  itself, another frame, an extension) is ignored. Answers go to that window
  with target `*`, the only way to address an opaque origin, which the
  pinned origin makes safe. The SDK sends only to the parent, under the
  Tokuly origin it found in `document.referrer` (the frame sends
  `referrerpolicy="origin"`) and which the mini app lists in `hostOrigins`,
  and reads only that window's messages from that origin.
- **Scopes.** Mini-app scopes, not API scopes, and no token: `profile`
  (name, username, picture) and `friends` (the same of friends the person
  picks each time). A call needs a scope the approved manifest asks for
  (`SCOPE_NOT_REQUESTED` otherwise) and that the person granted. The first
  such call asks once for every scope of the manifest; the grant is stored
  (`GrantMiniApp`) and withdrawn from the page's menu. The allow button
  works only after the sheet has been in front for a moment, because the
  mini app chooses when the sheet appears.
- **Sharing.** A card is posted only after the person sends it from a sheet
  that shows it, as an ordinary message from their session into the
  conversation the mini app was opened from: chat's own rules apply.
- One sheet at a time (`BUSY`). Location and coin payments are not part of
  the bridge.

## Bridge messages

Request `{ protocol: "tokuly.miniapp.v1", id, call }`, answered once with
`{ protocol, id, ok: true, result }` or `{ protocol, id, ok: false, error: { code, message } }`.

| `call.method` | Fields | Needs | Result |
|---|---|---|---|
| `hello` | — | — | `{ path, language, inConversation, grantedScopes }` |
| `getProfile` | — | `profile` | `{ id, name, handle, avatarUrl }` |
| `pickFriends` | `max` (1-20, default 20) | `friends` | the picked people, as `getProfile` |
| `shareCard` | `card: { title (1-60), text (≤200), path (on the mini app's origin, ≤500) }` | a conversation | `{ shared: true }` |
| `close` | — | — | `{ closed: true }`, then the page goes back to the chat |

Error codes: `INVALID_REQUEST`, `UNKNOWN_METHOD`, `SCOPE_NOT_REQUESTED`,
`CONSENT_DENIED`, `CANCELLED`, `NOT_IN_CONVERSATION`, `BUSY`, `UNAVAILABLE`.

In the mobile app's WebView the SDK sends through
`window.ReactNativeWebView.postMessage` and reads messages that no window
sent; the mobile host is not built yet.

## Development

```sh
TOKULY_DEV_MINI_APP_ALLOW_LOOPBACK=true   # the API: let a launch URL be http on 127.0.0.1
TOKULY_MINIAPP_HOSTS=http://localhost:53000 pnpm --filter @tokuly/miniapp-sample start
```

Register `http://127.0.0.1:5180/` as the launch URL, submit it, approve it
as a moderator, and send `<web origin>/miniapps/<app id>` into a chat. The
browser tests (`apps/web/e2e/miniapps.spec.ts`) start the same server.
