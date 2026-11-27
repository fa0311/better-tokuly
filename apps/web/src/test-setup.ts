import { loadAllStrings } from "#/lib/i18n";

// Outside a browser the app's `i18n` is the server's: every string, once loaded.
await loadAllStrings();
