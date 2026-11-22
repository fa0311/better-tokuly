/**
 * What the stories (記憶) feature shares with other features.
 *
 *   <StoryTray />                                   // the row of rings above a timeline
 *   <StoryRing user={user}>{avatar}</StoryRing>     // a face that plays the person's stories
 *   <ProfileHighlights user={user} isSelf />        // highlights on a profile
 *   openStoryViewer([{ user, stories }])            // play stories full screen from anywhere
 */
export { ProfileHighlights } from "./highlights";
export { StoryRing, StoryTray } from "./tray";
export { openStoryViewer, type ViewerReel } from "./viewer-store";
