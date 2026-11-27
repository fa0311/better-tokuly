# Web: the seams

A feature plugs into the app by adding files with fixed names to its folder;
the table of seam files and the rule are in
[`apps/web/README.md`](../../apps/web/README.md), "How a feature plugs in",
and each type is documented in `apps/web/src/seams/`. This page has examples
and how the pieces behave.

## Examples

```tsx
// src/features/plans/hub.tsx — a tile. `section` is one of the hub's groups:
// "play" (あそぶ), "out" (おでかけ), "make" (つくる・みる), "me" (アカウント).
export default [
  { id: "plans", section: "play", order: 20, to: "/plans", label: () => i18n.t("plans:tile"),
    icon: CalendarHeart, tint: "bg-lime", badge: "plans", keywords: ["予定", "calendar"] },
] satisfies HubTile[];

// src/features/school/hub-card.tsx — on the hub page, instead of the "school" tile:
// what class is on. It loads with the hub page and counts in its size, so keep it small.
export default function SchoolHubCard() { … }

// src/features/stories/nav.ts — /stories/* belongs to "notes" in the tab bar and the rail.
export default { primary: "notes", paths: ["/stories"] } satisfies NavClaim;

// src/features/plans/profile.tsx — slots: "status" (under the bio), "action" (a button),
// "card", "stat" (a cell of the numbers card), "content" (the lists at the bottom).
export default [
  { id: "plans", slot: "card", order: 10, Component: ({ profile, user, isSelf }) => (
      <ProfilePlans cards={profile.cards} user={user} isSelf={isSelf} className="mt-5" />
    ) },
] satisfies ProfileSection[];

// src/features/ask/badges.ts — `useBadge("ask")` is then this hook's result everywhere.
export default (() => useAskBadge()) satisfies BadgeSource;
```

## Forgetting a person

**Forgetting a person.** Unfriending and blocking call `forgetUser(queryClient, userId)`
from `#/seams/forget-user`, which runs every feature's `forget-user.ts`. A feature that
caches other people's content (timelines, trays, feeds) takes theirs out there, so
nothing of a blocked person keeps showing from the cache; one that cannot tell whose
an item is drops its lists and lets them load again.

## Badges

Badges are counts by key (`#/stores/badges`). A feature that already listens
for the events that change its count calls `setBadge("chat", n)` from its
`background.tsx`; one whose count simply follows its queries exports a hook
from `badges.ts`, which counts the badge named after the area and starts the first
time a screen shows it. Tiles and navigation items name the key they show.

## Navigation

**Navigation.** The primary destinations (map, chat, notes, watch) are one
tap away in the phone tab bar and the rail (`components/app-shell.tsx`).
Everything else is opened from the hub (`/me`, the last tab on phones, with
search on top), the launcher (the rail's "アプリ") and the quick search
(Cmd/Ctrl+K, which also finds hub tiles by name and keyword). A new area
needs a `hub.tsx` to be reachable and a `nav.ts` when its routes belong
under a primary item; pages nobody claims light the hub's slot.

A page that needs the phone's whole height (a conversation, a full-screen
player) calls `useHideTabBar()` from `#/stores/shell` instead of covering
the tab bar. Toasts appear above the tab bar on phones and at the top on
wide screens; do not move them per screen.
