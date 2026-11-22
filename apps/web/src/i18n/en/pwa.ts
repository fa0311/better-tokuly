export default {
  title: "Use as an app",
  // The row in the settings list that leads here.
  settingsRow: { title: "Use as an app", hint: "Home Screen, offline, storage" },

  banner: {
    label: "Add to Home Screen",
    title: "Add to your Home Screen!",
    body: "Opens in a tap, with notifications",
    inAppBody: "Open it in Safari to add it",
    install: "Add",
    howTo: "How",
    dismiss: "Later",
  },

  install: {
    installedTitle: "You're using the app!",
    installedBody: "Open it from the icon on your Home Screen whenever you like.",
    justInstalledTitle: "Added to your Home Screen!",
    justInstalledBody: "From now on, open Tokuly from its icon on your Home Screen.",
    title: "Add Tokuly to your Home Screen!",
    perks: {
      fast: "One tap on the icon and it's open",
      push: "Notifications when your friends are up to something",
      calm: "Keeps its cool on a weak connection",
    },
    install: "Install",
    howTo: "Show me how to add it",
    hints: {
      menu: "Pick “Install app” or “Add to Home screen” from your browser's menu.",
      macDock: "In Safari's menu bar, choose File → “Add to Dock”.",
      none: "This browser can't install apps. Try Safari on iPhone, or Chrome on Android and computers.",
    },
    installedToast: "Added to your Home Screen! 🎉",
  },

  guide: {
    title: "Adding Tokuly to your Home Screen",
    lead: "Three steps and Tokuly works like an app!",
    inApp: {
      title: "First, open it in Safari",
      body: "It can't be added from inside this app. Pick “Open in Safari” from the menu, or copy the link and paste it into Safari.",
      copy: "Copy link",
      copied: "Copied! Paste it into Safari",
      copyFailed: "Couldn't copy… press and hold the address to copy it",
    },
    steps: {
      share: {
        title: "Tap the Share button",
        safari: "It's in Safari's bar. If you can't see it, look inside “…”.",
        browser: "It's next to the address bar.",
      },
      add: { title: "Choose “Add to Home Screen”", body: "Scroll down to find it." },
      confirm: { title: "Tap “Add” in the top right", body: "And Tokuly is on your Home Screen!" },
    },
    // Labels inside the little pictures of the steps, as iOS shows them.
    art: { copy: "Copy", addToHome: "Add to Home Screen", bookmark: "Add Bookmark", cancel: "Cancel", add: "Add" },
    done: "Got it!",
  },

  status: {
    title: "Right now",
    install: {
      label: "Installed",
      yes: "Yes",
      no: "Not yet",
      yesHint: "You're looking at the installed app.",
      noHint: "You're looking at Tokuly in a browser.",
    },
    push: {
      label: "Push notifications",
      on: "On",
      off: "Off",
      blocked: "Blocked",
      unsupported: "Not supported",
      onHint: "This device gets your notifications even when Tokuly is closed.",
      offHint: "Turn them on to hear from your friends right away.",
      blockedHint: "Your browser's settings block notifications for this site.",
      unsupportedHint: "This browser doesn't support push notifications.",
      needsInstallHint: "On iPhone and iPad, notifications work once Tokuly is on your Home Screen.",
      open: "Open notification settings",
    },
    offline: {
      label: "Offline ready",
      ready: "Ready",
      preparing: "Getting ready",
      unsupported: "Not supported",
      readyHint: "Lose your connection and a waiting screen takes over, then brings you back by itself.",
      preparingHint: "Almost ready.",
      unsupportedHint: "Without a connection, this browser shows its own error page.",
    },
    version: {
      label: "Version",
      latest: "Up to date",
      update: "Update ready",
      latestHint: "You're on the newest Tokuly.",
      updateHint: "There's a new version. Updating reloads this screen.",
      check: "Check",
      apply: "Update",
      upToDate: "You're on the newest version!",
    },
  },

  storage: {
    title: "Storage",
    lead: "How much Tokuly keeps on this device.",
    used: "in use",
    of: "of the {{quota}} it may use",
    meter: "Storage in use",
    unknown: "This browser doesn't say how much is in use.",
    files: "Files saved to open faster: {{count}}",
    clear: "Clear offline data",
    clearTitle: "Clear offline data?",
    clearBody: "This removes the files saved to open faster and frees up space. Your account and messages stay. Only the next start may take a little longer.",
    clearConfirm: "Clear",
    cleared: "Offline data cleared!",
    clearFailed: "That didn't work… try again",
  },

  update: {
    toast: "There's a new version!",
    reload: "Update",
    later: "Later",
  },

  offline: {
    label: "You're offline",
  },
} as const;
