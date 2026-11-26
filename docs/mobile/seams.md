# Mobile: the seams

A feature plugs into the app by adding files with fixed names to its folder;
the table of seam files and the rule are in
[`apps/mobile/README.md`](../../apps/mobile/README.md), "How a feature plugs
in", and each type is documented in `apps/mobile/src/seams/`. This page has
the example, the reasoning behind the tab bar and how the registries are built.

## An example

```tsx
// src/features/plans/tabs.tsx — with app/(app)/(tabs)/plans.tsx rendering the screen.
export default [
  { name: "plans", order: 35, title: () => i18n.t("plans:tab"), icon: CalendarHeart, badge: "plans" },
] satisfies TabEntry[];

// src/features/plans/badges.ts — the tab's counter.
export default { plans: () => useInvitations().data?.length ?? 0 } satisfies BadgeSources;

// src/features/plans/screens.ts — app/(app)/plans/new.tsx opens as a modal.
export default [
  { name: "plans/new", options: () => ({ title: i18n.t("plans:new"), presentation: "modal" }) },
] satisfies ScreenEntry[];

// src/features/plans/links.ts — a push about a plan opens it.
export default [
  { path: /^\/plans\/([^/]+)$/, screen: (match) => `/plans/${match[1]}` },
] satisfies LinkTarget[];
```

## The tab bar

The bar holds six at most. Today: マップ (the heart: where the app opens),
チャット, ノート, ショート, お知らせ, じぶん — what a high schooler opens many
times a day, and the two places things arrive (the chats' and the inbox's
counters). Everything else is an entry of the hub under the own profile:
ともだち first (the list, requests, adding someone: friends themselves are met
on the map and in the chats), 「もっと見る」, the settings. An entry names the
count that waits behind it (`badge`), the entry shows it and the じぶん tab
shows the sum, so a friend request is still seen from the bar (and it is an
inbox entry and a push as well). A new area that wants a tab has to make the
case against one of the six; `src/seams/__tests__/seams.test.ts` holds the line.

## The registries

Metro has no `import.meta.glob`, so `scripts/seams.js`
writes the list of each seam's files to `src/generated/seams/<seam>.ts`: from
`metro.config.js` (every bundle and dev server) and before `typecheck` and
`test`. They are not checked in, like the web's route tree: two branches that
both add a feature would always conflict there. After adding or removing a
seam file while Metro runs, `pnpm --filter @tokuly/mobile seams`; a jest run
on stale registries fails (`src/seams/__tests__/registry.test.ts`).
A file that carries a seam's name but would not be listed (deeper than the
feature's folder, not `.ts(x)`, no `export default`) stops the generator, and
with it the bundle: a `lifecycle.ts` left out would be a privacy hook that
never runs.
(`require.context` would do the same inside Metro, but it is still behind an
`unstable_` flag, jest needs a stand-in for it, and its result is untyped: a
generated import list is checked by `tsc` against each seam's type.)
