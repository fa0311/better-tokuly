import type { NavClaim } from "#/seams/nav";

/** Live streams and clips sit under "watch", next to videos. (`/studio/live` is the video studio's claim already.) */
export default { primary: "watch", paths: ["/live", "/clip"] } satisfies NavClaim;
