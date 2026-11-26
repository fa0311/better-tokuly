import { PermissionsAndroid } from "react-native";

/** Android asks for the microphone at run time, before a room opens it. */
export async function microphoneAllowed(): Promise<boolean> {
  const permission = PermissionsAndroid.PERMISSIONS.RECORD_AUDIO;
  if (await PermissionsAndroid.check(permission)) return true;
  return (await PermissionsAndroid.request(permission)) === PermissionsAndroid.RESULTS.GRANTED;
}
