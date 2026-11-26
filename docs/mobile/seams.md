# Mobile: the seams

A feature plugs into the app by adding files with fixed names to its folder;
the table of seam files and the rule are in
[`apps/mobile/README.md`](../../apps/mobile/README.md), "How a feature plugs
in", and each type is documented in `apps/mobile/src/seams/`. This page has
an example, the tab bar's rule and how the registries are built.

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

The bar holds six at most. Today: マップ (where the app opens), チャット,
ノート, ショート, お知らせ, じぶん. Everything else is an entry of the hub
under the own profile (ともだち first, 「もっと見る」, the settings). An entry
names the count that waits behind it (`badge`); the じぶん tab shows the sum.
A new area that wants a tab has to make the case against one of the six;
`src/seams/__tests__/seams.test.ts` holds the line.

## The registries

Metro has no `import.meta.glob`, so `scripts/seams.js` writes the list of
each seam's files to `src/generated/seams/<seam>.ts` (from `metro.config.js`
and before `typecheck` and `test`; not checked in). After adding or removing
a seam file while Metro runs: `pnpm --filter @tokuly/mobile seams`. A file
that carries a seam's name but would not be listed (deeper than the
feature's folder, not `.ts(x)`, no `export default`) stops the generator: a
`lifecycle.ts` left out would be a privacy hook that never runs.
