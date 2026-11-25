// The background location task is defined before anything else: the OS may start the
// app without a screen just to deliver fixes (Android headless JS, iOS relaunch).
import "#/features/map/background-report";
import "expo-router/entry";
