/**
 * What the settings feature shares with other features.
 *
 * The account's settings are one cached query: read them with `useSettings`
 * (privacy and the preferences — distance unit, map style, autoplay, sounds,
 * haptics) and they stay current when the person changes them in the
 * settings screens. `useAppearance` is the theme and ドパガキ度 in effect in
 * this browser.
 */

export { enterAccount } from "./accounts/accounts-section";
export { DeviceAccountsOnLogin } from "./accounts/device-accounts-on-login";
export { type Appearance, useAppearance } from "./appearance/appearance-store";
export { BirthdateGate } from "./birthdate/birthdate-prompt";
export { usePasskeysAvailable, useSettings } from "./data";
