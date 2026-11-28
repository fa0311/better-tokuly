# Tokuly web

TanStack Start (React 19, Vite 8, React Compiler) · Tailwind 4 · Base UI ·
MapLibre GL · Connect RPC.

```sh
pnpm install
pnpm --filter @tokuly/api run generate   # TypeScript API from proto/
pnpm --filter @tokuly/web dev            # http://localhost:3000
pnpm --filter @tokuly/web e2e            # browser tests against a running stack
```

The dev server forwards `/api/*`, `/media/*`, `/tiles/*` and `/map-assets/*`
(`TOKULY_DEV_API_URL`, `TOKULY_DEV_MEDIA_URL`, `TOKULY_DEV_TILES_URL`,
`TOKULY_DEV_MAP_ASSETS_URL`), so the browser only ever talks to one origin —
the same as behind the edge proxy.

## Layout

```
src/routes/            file-based routes. `_app/*` needs a signed-in user (client-rendered).
src/features/<area>/   one folder per product area: screens, components, hooks, clients
src/components/ui/     design-system primitives (see below)
src/components/        app-wide pieces (AppShell, the hub and launcher, AuthLayout, BackgroundFeatures)
src/seams/             where features plug into the app (see below)
src/lib/               api, realtime, i18n, formatting, errors, appearance, upload, account time
src/stores/            zustand stores for live state (session, locations, badges, shell)
src/i18n/{ja,en}/      strings of the web-only areas, one file (= namespace) per feature area
e2e/                   Playwright tests, one `<area>.spec.ts` per feature area
```

A route file stays thin: it declares the route and renders a screen from
`src/features/`. Nested pages are flat files: `stamps.pack.$packId.tsx` is
`/stamps/pack/:packId`. `src/routeTree.gen.ts` is written by the dev server and
by `vite build` from the route files and is not checked in (two branches that
both add a route would otherwise always conflict there): on a fresh checkout,
start the dev server or build once before `typecheck`.

What a feature shares with other features is exported from
`src/features/<area>/index.ts` (for example the stamp picker); everything
else in the folder is private to it.

## Shared with the other apps

The mobile and desktop apps run the same logic as this one. It lives in
`packages/` ([`packages/README.md`](../../packages/README.md)), not here:
the strings of the areas every app has in `@tokuly/i18n` (`src/i18n/{ja,en}.ts`
merge them with the web-only namespaces), query keys and options, domain
rules, cache edits, formatting and error texts in `@tokuly/core/<area>`.
Only logic is shared, never UI. A feature folder keeps what is the web's:
hooks, stores, screens, components, the browser's APIs. Where shared logic
speaks or fetches, it takes the web's i18next instance or client as an
argument (`#/lib/format`, `#/lib/errors`). Logic moves there with its tests.

## How a feature plugs in

A feature never edits the hub, the profile screen, the navigation, the badge
store, the settings list or the staff console. It adds a file with a fixed
name to its own folder and default-exports what it contributes; the app
collects the files (`import.meta.glob` in `src/seams/*`, where each type is
documented). Two features can therefore be built side by side without
touching the same file.

| file in `src/features/<area>/` | default export                          | what it does                                                                          |
| ------------------------------ | --------------------------------------- | ------------------------------------------------------------------------------------- |
| `background.tsx`               | a component (usually rendering nothing) | runs on every page while someone is signed in: realtime listeners, toasts, ringing; what reacts to a frame loads with it (`onFramesLoading`) |
| `hub.tsx`                      | `HubTile[]` (`#/seams/hub`)             | the area's way in: a tile on the hub (`/me`), in the launcher and in the quick search |
| `hub-card.tsx`                 | a component (`#/seams/hub-cards`)       | a widget on the hub page instead of the area's tile; loads with the hub page only     |
| `nav.ts`                       | `NavClaim` (`#/seams/nav`)              | which primary item (map, chat, notes, watch) is lit on the area's routes              |
| `profile.tsx`                  | `ProfileSection[]` (`#/seams/profile`)  | what the area shows on a person's profile, by slot                                    |
| `badges.ts`                    | `BadgeSource` (`#/seams/badges`)        | the area's badge, from its queries; starts once a screen shows it                    |
| `settings-section.ts`          | `SettingsSection` (`#/seams/settings`)  | a row of `/settings`                                                                  |
| `admin-section.ts`             | `AdminSection` (`#/seams/admin`)        | an entry of the staff console's navigation                                            |
| `channel-tab.tsx`              | `ChannelTab` (`features/video`)         | a tab on every channel page                                                           |
| `forget-user.ts`               | `ForgetUser` (`#/seams/forget-user`)    | after an unfriend or a block: that person's content leaves the area's cached queries  |
| `sign-out.ts`                  | `SignOutCleanup` (`#/seams/sign-out`)   | the account's storage entries and in-memory state, removed when its session ends      |

- A new area needs a `hub.tsx` to be reachable, and a `nav.ts` when its
  routes belong under a primary item (map, chat, notes, watch).
- Unfriending and blocking call `forgetUser(queryClient, userId)`
  (`#/seams/forget-user`): a feature that caches other people's content
  takes theirs out in its `forget-user.ts`, or drops its lists.
- Whatever a feature keeps of the account outside the query cache
  (`localStorage`, `sessionStorage`, stores) goes in its `sign-out.ts`;
  `src/seams/sign-out.test.ts` fails on a storage key that is neither there
  nor kept on purpose.
- A page that needs the phone's whole height calls `useHideTabBar()`
  (`#/stores/shell`) instead of covering the tab bar. Toasts are not moved
  per screen.

Examples, badges and navigation: [`docs/web/seams.md`](../../docs/web/seams.md).

**Between features.** Before writing your own, use what is there:

- text people wrote (messages, notes, comments, names, bios): `<EmojiText>`;
  a composer: `<EmojiSuggestBar>`; choosing an emoji: `<EmojiPicker>`
  (`unicodeOnly` where the RPC takes no `:shortcode:`), all from `features/emoji`;
- a share control: `<ShareButton subject>`; a menu of your own:
  `useSubjectMenu(subject)` adds "send in chat", "copy link" and "report";
  a header without a menu: `<SubjectMoreButton>` (`features/engagement`);
- a map: `<MapView>` / `<MapFrame>`, a picture of a place:
  `<StaticMapPreview>` (`features/map`; import it with `lazy()` outside map
  screens, the map library is large);
- "today" and "now": `#/lib/account-time` (the account's time zone), never the
  device's calendar, for anything compared with the API's dates;
- returning a visitor of a public page after sign-up or sign-in: link to
  `/signup` or `/login` with `search={{ redirect: "/your/page" }}`.

Primitives in `components/ui`: `Button`, `TextField` / `TextArea`,
`SelectField`, `SwitchRow`, `Tabs`, `Dialog` (a sheet on phones),
`ConfirmDialog`, `Menu`, `Card` / `Panel` / `Chip`, `Avatar`, `CountBadge`,
`Coins` / `CoinIcon`, `Page` / `EmptyState` / `ListRow` / `Skeleton`,
`LoadMore`, `Spinner` / `PaneSpinner`. Toasts are `toast` from `sonner`.

## Conventions

**Data**

- Typed clients come from `api().service(<Service>)` (`#/lib/api`), with the
  generated messages under the domain namespaces of `@tokuly/api`
  (`social.FriendService`, `geo.FriendLocation`, ...).
- Reads go through TanStack Query (`useQuery` / `useInfiniteQuery` with the
  response's `page.nextCursor`). Query keys start with the domain:
  `["social", "friends"]`. Mutations invalidate or update those keys.
- Live updates come from the single realtime connection (`realtime()` in
  `#/lib/realtime`): listen with `onFrame`, subscribe to topics with
  `subscribe(topic)` and release on unmount. Push into the query cache or a
  zustand store; never poll.
- Timestamps: `toDate` / `toMillis` from `@tokuly/api`. IDs are strings.
- Errors shown to people go through `errorMessage(error)` (`#/lib/errors`),
  which maps the API's reason code to text. Every reason has a text in the
  always-loaded `errors` namespace (`packages/i18n`; a test checks it against
  the protos and the backend). A feature that words one its own way puts it
  under `errors` in its namespace and names itself:
  `errorMessage(error, { namespace: "stamps" })`.
- Infinite lists: `useInfiniteQuery` + `<LoadMore query={query} />` after the
  last row.
- Uploads: `uploadAndProcess(file, { purpose })` from `#/lib/upload`.
- Unread counters: `setBadge("notifications", n)` from `#/stores/badges`, or
  a hook in the feature's `badges.ts`.

**UI**

- The look is "sticker pop" (see `src/styles.css`): `sticker` (ink outline +
  hard shadow) for standalone objects, `pressable` for things you can press,
  `glass` for chrome floating above the map. Colours are the semantic tokens
  (`bg-surface`, `text-ink-soft`, `bg-brand`, `bg-lime`...), never raw hex in
  components (ink-on-colour text `text-[#17122b]` on lime/sun/sky is the one
  exception). Both themes must work: check `data-theme="dark"`.
- Use the primitives in `components/ui` before writing new ones; extend them
  when a pattern repeats. Non-map screens use `<Page>`.
- Motion: CSS animations from the theme (`animate-pop-in`, `animate-slide-up`)
  or `motion`. Durations scale with `var(--dopa)` (ドパガキ度); at 0 and under
  `prefers-reduced-motion` everything must still work without movement.
- Phones first: every screen works at 360 px wide, with the floating tab bar
  (`<Page>` leaves room for it and for a toast above it; a page with its own
  scroller leaves `pb-44`) and safe-area insets.
  Wide screens get the rail and more columns.
- Accessibility: real buttons/links, labels on icon buttons (`aria-label`),
  visible focus (built in), no information by colour alone.
- No manual `useMemo`/`useCallback`/`memo`: the React Compiler memoizes.

**Security headers**

- Every page gets a Content-Security-Policy and a Permissions-Policy
  (`src/lib/security-headers.ts`, applied in `src/start.ts`): scripts run
  from this origin or with the response's nonce only, no inline handlers or
  `eval`. A new site the browser must reach (a player, a service) or a new
  browser feature is added there. The browser tests fail on any violation.
- Addresses of the deployment (realtime, LiveKit, the media server,
  uploads) come from the backend's `TOKULY_*` variables, which Compose hands
  the web server too: set them in `.env`, not only in `tokuly.toml`.

**Text**

- Every string goes through `useTranslation("<namespace>")`, in Japanese
  (casual and friendly, like 「友達と位置情報共有を楽しもう!」) and English.
  A file in `src/i18n/ja/` and its twin in `src/i18n/en/` are a namespace
  named after the file; nothing needs registering. The browser loads only
  the language it shows, and a namespace when a component first asks for it
  (`#/lib/i18n`). The namespaces of the
  areas every app has are in `packages/i18n/src/{ja,en}/` (listed in its
  `index.ts`).
- `t` is new whenever any namespace loads, so an effect never lists it (a lint
  rule, `lint/no-t-in-effect-deps.grit`): it reads its strings with
  `i18n.getFixedT(null, "<namespace>")`, or keeps keys and translates where
  they are shown.

**Checks**

- `pnpm --filter @tokuly/web typecheck` and `test` must pass; `vite build`
  must succeed, and after it `pnpm --filter @tokuly/web size --check`: what
  the first page of a route downloads stays within `startup-budget.json`.
  Anything outside a route's `component` (`validateSearch`, `beforeLoad`,
  `head`) is in every page's download: take it from a small module
  (`<screen>-search.ts`), never from a screen. Changes to `packages/` also need
  `pnpm --filter "./packages/*" test` and `typecheck`.
- Formatting and lint: `pnpm lint:web` (repo root; Biome, line width 100,
  sorted imports) must be clean — CI checks it. `pnpm format:web` fixes what
  can be fixed automatically; run it before committing.
- Every feature has browser tests in `e2e/<area>.spec.ts` (Playwright, see
  `e2e/support.ts`): they sign up their own accounts, arrange state through
  the API with `rpc(...)`, and drive the real UI at phone and desktop size.
  `TOKULY_WEB_URL` points them at the stack (default http://localhost:3000).
- Look at your screens in a real browser at phone and desktop width, light
  and dark: `node tools/dev/shot.mjs --url /stamps --user demo --signup --out shot.png`
  (repo root) saves a screenshot. The map needs WebGL2.
