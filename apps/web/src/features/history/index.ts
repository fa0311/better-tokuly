/**
 * What the history feature shares with other features:
 *
 *   <Link to="/history/$date" params={{ date: formatDay(new Date()) }} />   // a day's diary page
 *   decodePolyline(day.pathPolyline)                                       // [{ lat, lng }, …] of an encoded path
 */
export { formatDay, parseDay } from "./dates";
export { decodePolyline } from "./timeline";
