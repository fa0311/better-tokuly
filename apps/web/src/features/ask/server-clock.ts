/**
 * The server's clock, as far as this device knows it. Deadlines the server
 * sends (when the next round opens) are counted down against its time, not
 * the device's: a phone whose clock is off would otherwise show the wrong
 * wait, or offer a round the server does not have yet.
 *
 * The offset comes from the realtime connection, whose welcome and pong
 * frames carry the server time (`background.tsx` passes them on). Until one
 * arrived the device clock is used as it is.
 */
let offsetMs = 0;

/** Remembers how far the server's clock is from this device's. */
export function noteServerTime(serverMs: number, deviceMs: number = Date.now()): void {
  if (Number.isFinite(serverMs) && serverMs > 0) offsetMs = serverMs - deviceMs;
}

/** The current time on the server's clock, in milliseconds. */
export function serverNow(deviceMs: number = Date.now()): number {
  return deviceMs + offsetMs;
}
