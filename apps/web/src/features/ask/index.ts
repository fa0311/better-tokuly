/**
 * What the question-box feature (質問箱・お題・ほめ投票) shares with other features.
 *
 *   <ProfileAskButton user={user} isSelf={isSelf} />   // "質問する" on a profile (the viewer's own: "質問箱")
 *   <ProfileAnswers user={user} />                     // the newest published Q&A of a person
 *   <QaCard question={question} />                     // one answered question anywhere: like, comments, share as an image
 *   useAskBadge()                                      // unread questions + unseen flames, for a badge on the way in
 *   boxShareUrl(handle)                                // the public page of a box: `/q/<handle>`
 *
 * Addresses: `/ask` (hub), `/ask/u/<handle>` (a person's box in the app),
 * `/ask/q/<question id>`, `/ask/prompts/<prompt id>`, `/ask/inbox`,
 * `/ask/flames`, `/ask/polls`, and the public `/q/<handle>`.
 */
export { boxShareUrl } from "./model";
export { ProfileAnswers, ProfileAskButton, useAskBadge } from "./profile-ask";
export { QaCard } from "./qa-card";
