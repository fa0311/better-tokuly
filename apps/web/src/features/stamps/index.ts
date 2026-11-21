/**
 * What the stamps feature shares with other features. Chat, live chat,
 * comments, notes and the map show stamps with `StampImage`, let people
 * choose one with `StampPicker`, and open `StampSheet` when a stamp someone
 * sent is tapped (details, favourite, "save this stamp").
 *
 *   <StampImage stamp={message.stamp} size={128} />
 *   <StampPicker trigger={<Button size="icon" aria-label="スタンプ">…</Button>} onPick={(stamp) => send({ stampId: stamp.id })} />
 *   <StampSheet stampId={openStampId} onClose={() => setOpenStampId(undefined)} />
 */
export { StampPicker, type StampPickerProps } from "./picker";
export { StampImage, type StampImageProps } from "./stamp-image";
export { StampSheet, type StampSheetProps } from "./stamp-sheet";
