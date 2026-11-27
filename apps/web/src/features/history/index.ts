/**
 * What the history feature shares with other features:
 *
 *   <Link to="/history/$date" params={{ date: accountToday() }} />   // a day's diary page (`#/lib/account-time`)
 *   decodePolyline(day.pathPolyline)                                       // [{ lat, lng }, …] of an encoded path
 */
export { decodePolyline, formatDay, parseDay } from "@tokuly/core/history";
