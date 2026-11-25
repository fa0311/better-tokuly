/** The screens only the phone app has: tabs, location permission, background sharing, settings. */
export default {
  tabs: { map: "Map", friends: "Friends", me: "Me" },
  welcome: {
    signUp: "Get started",
    signIn: "Sign in",
  },
  accounts: {
    continueTitle: "Accounts on this device",
    continue: "Continue as @{{handle}}",
    signInAnother: "Sign in to another account",
  },
  sharing: {
    explain: {
      title: "Share where you are!",
      body: "Turn it on and you show up on your friends' map. Blur or freeze yourself with ghost mode, and stop any time.",
      whileUsing: "First pick “Allow While Using App”",
      always: "Want your friends to see you while the app is closed? Then pick “Always”!",
    },
    turnOn: "Turn on",
    notNow: "Not now",
    sharing: {
      title: "Sharing your now",
      foreground: "Friends see where you are while the app is open.",
      background: "Friends get your now even while the app is closed.",
    },
    alwaysAsk: "Keep sharing when closed",
    alwaysHint: "Pick “Always” on the next screen",
    approximate:
      "Only your approximate location is allowed. Turn on “Precise Location” so friends see you properly.",
    blocked: {
      title: "Location is off",
      body: "In the Settings app, set Tokuly's location to “While Using” or “Always”.",
    },
    servicesOff: {
      title: "Location services are off",
      body: "Turn on location services in Control Center or Settings.",
    },
    openSettings: "Open Settings",
    stopped: { title: "Sharing paused", body: "Friends see your last place." },
    resume: "Resume",
    stop: "Stop sharing",
    notification: {
      title: "Sharing your now",
      body: "Your friends' map shows where you are",
    },
  },
  map: {
    ghost: "Ghost",
    sharingOff: "Sharing off",
    friendsOnMap: "{{count}} on the map",
  },
  friends: {
    add: "Add friends",
    addTitle: "Add friends",
    byHandle: "Add by handle",
    handle: "Handle",
    handlePlaceholder: "e.g. tokuly_chan",
    send: "Send request",
    byInvite: "Add by invite link",
    inviteLink: "Invite link or code",
    invitePlaceholder: "https://…/invite/xxxx",
    open: "Open",
    myInvite: "Your invite link",
    shareInvite: "Share invite link",
    shareMessage: "Be my friend on Tokuly! {{url}}",
    invitedBy: "{{name}} invited you!",
    inviteLead: "As friends you see each other's now on the map.",
    alreadyFriends: "You're friends already!",
    invalidInvite: "This invite link doesn't work",
  },
  settings: {
    title: "Settings",
    accounts: "Switch accounts",
    web: "More settings",
    webHint: "Settings the app doesn't have yet open on the web",
    webNote:
      "Sign in on the web again. For your safety the app never hands its sign-in to the web.",
    signOut: "Sign out",
    signOutConfirm: "Sign this account out of this device? Your other accounts stay.",
    version: "Version {{version}}",
    server: "Server {{url}}",
  },
  profile: {
    me: "Me",
    viewOnWeb: "Open on the web",
  },
} as const;
