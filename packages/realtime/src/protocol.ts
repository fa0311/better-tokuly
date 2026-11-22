/**
 * The version of the realtime protocol this client was built for
 * (`Welcome.protocol_version`, see `realtime.proto`; the gateway's number is
 * `PROTOCOL_VERSION` in `backend/crates/realtime/src/frames.rs`).
 */
export const PROTOCOL_VERSION = 2;

/**
 * Whether a server announcing `serverVersion` has moved past this client.
 * Such a client still connects, but no longer understands everything it is
 * sent: the page has to be loaded again to get the client that does. An
 * older server (0 before versions existed) is one that is still being
 * deployed, and reloading would not change what it sends.
 */
export function clientIsOutdated(serverVersion: number): boolean {
  return serverVersion > PROTOCOL_VERSION;
}
