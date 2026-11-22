import type { NavClaim } from "#/seams/nav";

/** Everything about video sits under "watch". */
export default {
  primary: "watch",
  paths: ["/watch", "/c", "/playlist", "/studio", "/party"],
} satisfies NavClaim;
