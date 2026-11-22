import type { NavClaim } from "#/seams/nav";

/** Notes, tag pages, friends' activity and the monthly recap sit under notes. */
export default {
  primary: "notes",
  paths: ["/notes", "/tag", "/activity", "/recap"],
} satisfies NavClaim;
