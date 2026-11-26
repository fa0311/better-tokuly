# tokuly-web-data

A local Expo module (autolinked from `apps/mobile/modules`) that removes
everything web views have stored on the device: cookies, DOM storage
(localStorage, IndexedDB, …) and the cache. The JS side is
`src/platform/web-data.ts`; the web screens (`src/features/web`) call it
before a web view opens, when it closes, and before a switch or a sign-out,
so nothing one account's web session left reaches another account.

- Android: `CookieManager.removeAllCookies` (then `flush`, so the store on
  disk is empty too), `WebStorage.deleteAllData`, `WebView.clearCache(true)`.
  The web views' incognito mode only empties the cookies when one opens.
- iOS: `WKWebsiteDataStore.default()` loses every data type. The web views
  are incognito there (a store of their own, never written), so this is the
  belt to those braces.

No third-party code: the platforms' own APIs only.
