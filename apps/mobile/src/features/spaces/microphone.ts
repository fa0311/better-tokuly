/** iOS asks for the microphone by itself the first time a room opens it. */
export async function microphoneAllowed(): Promise<boolean> {
  return true;
}
