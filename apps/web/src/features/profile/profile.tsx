import type { ProfileSection } from "#/seams/profile";
import { ProfileBook } from "./profile-book";

/** The プロフ帳 on every profile, after the cards of the other features. */
export default [
  { id: "profile-book", slot: "card", order: 90, Component: ProfileBook },
] satisfies ProfileSection[];
