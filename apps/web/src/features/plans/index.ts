/**
 * What the plans area ("あそぶ": free-time status, plans, date polls,
 * countdowns) shares with other features.
 *
 *   <FreeFriendsStrip />                      // who is free right now; "ひま!" and "あそぼ!" in one tap
 *   <FreeFriendsStrip bare className="…" />   // without its heading, above a map or a chat list
 *   <FreeFriendsStrip bare quiet />           // as a guest on another screen: one slim row while nobody is free
 *   <ProfilePlans cards={profile.cards} user={profile.user} isSelf={isSelf} />   // ひま, pinned countdowns, the couple counter
 *   startPlan({ inviteeIds: [user.id] })      // then navigate to `/plans/new`: the form opens prefilled
 *
 * The strip keeps itself current from the realtime connection and needs
 * nothing around it; it leads to `/plans/free`.
 */
export { ProfilePlans, type ProfilePlansProps } from "./countdown/profile-cards";
export { type PlanDraft, setPlanDraft as startPlan } from "./draft";
export { FreeFriendsStrip, type FreeFriendsStripProps } from "./free/free-strip";
