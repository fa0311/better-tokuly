# Shared packages

Code that every Tokuly app (web, mobile, desktop) runs the same way.

| package            | what it holds                                                                         |
| ------------------ | ------------------------------------------------------------------------------------- |
| `@tokuly/api`      | the generated API (Protobuf + Connect RPC), the session and its token refresh, errors |
| `@tokuly/realtime` | the realtime connection (WebTransport, WebSocket fallback) and its frames             |
| `@tokuly/i18n`     | the strings of the areas every app has (ja first, en), the language rules             |
| `@tokuly/core`     | app logic without UI or platform: one entry point per area                            |

`@tokuly/core` has one entry point per area (`@tokuly/core/chat` is
`core/src/chat/index.ts`), each with its own tests; what each holds is in
[`docs/packages.md`](../docs/packages.md).

## What belongs here

> 「共通化の為に責務が多いコンポーネントになったりするのは本質的じゃない。工数を少なく出来るのは
> 嬉しいが、バグが増えたり保守性も考慮するべきだ」 — the product owner

Logic only:

- query keys, query options (`{ queryKey, queryFn, ... }`, no hooks) and mutation functions;
- pure domain rules: who may do what, which list something is in, unread counting, grouping,
  validation, limits;
- formatting (dates, distances, counts, relative times) and error reason → message;
- strings (`@tokuly/i18n`);
- cache edits for what the user does and what realtime events report (functions of a
  `QueryClient` from `@tanstack/query-core`, or pure functions of the cached data);
- planning that does not touch a platform (which slides to load, how to page).

Never:

- UI of any kind: no React components, no hooks, no "universal" components;
- the platform: no `window`, `document`, `navigator`, storage, cookies, `Platform.OS`, file
  pickers, and no branches on which platform this is. When logic needs something only the
  platform has, the function takes the value as a plain argument (the clock is fine);
- set-up state: no global singletons configured at start-up, no adapter registries. A function
  that speaks takes the app's i18next instance (`errorMessage(i18n, error)`,
  `chatTimes(i18n)`); a query takes the app's client as a `ClientSource` (`() => client`);
- anything from `apps/*`, a router or React's DOM.

Each module has one job and a small public surface. When one grows a second job, split it.
Small duplication in app code is better than a shared abstraction with an option per platform.
Move something here when a second app needs it, not before.

`packages/core/src/boundaries.test.ts` fails when a shared package imports UI, a router, an app
or a platform, touches browser globals, or its `tsconfig.json` includes the DOM library.
