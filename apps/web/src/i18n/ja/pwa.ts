export default {
  title: "アプリとして使う",
  // The row in the settings list that leads here.
  settingsRow: { title: "アプリとして使う", hint: "ホーム画面に追加・オフライン・容量" },

  banner: {
    label: "ホーム画面に追加",
    title: "ホーム画面に追加しよう！",
    body: "すぐ開けて、通知も届くよ",
    inAppBody: "Safariで開くと追加できるよ",
    install: "追加する",
    howTo: "やり方",
    dismiss: "あとで",
  },

  install: {
    installedTitle: "アプリとして使ってるよ！",
    installedBody: "ホーム画面のアイコンから、いつでもすぐ開けるよ。",
    justInstalledTitle: "ホーム画面に追加できたよ！",
    justInstalledBody: "次からは、ホーム画面のTokulyアイコンから開いてね。",
    title: "ホーム画面に追加しよう！",
    perks: {
      fast: "タップひとつで、すぐ開ける",
      push: "友達からのお知らせが通知で届く",
      calm: "電波が弱くても、あわてない",
    },
    install: "インストールする",
    howTo: "追加のしかたを見る",
    hints: {
      menu: "ブラウザのメニューから「アプリをインストール」か「ホーム画面に追加」をえらんでね。",
      macDock: "Safariのメニューバーで「ファイル」→「Dockに追加」をえらんでね。",
      none: "このブラウザからはインストールできないみたい。iPhoneはSafari、Androidやパソコンは Chrome で開いてみてね。",
    },
    installedToast: "ホーム画面に追加したよ！🎉",
  },

  guide: {
    title: "ホーム画面に追加するには",
    lead: "3ステップで、アプリみたいに使えるよ！",
    inApp: {
      title: "まずはSafariで開こう",
      body: "いま開いているアプリの中からは追加できないんだ。メニューの「Safariで開く」をえらぶか、リンクをコピーしてSafariに貼りつけてね。",
      copy: "リンクをコピー",
      copied: "コピーしたよ！Safariに貼りつけてね",
      copyFailed: "コピーできなかった…アドレスを長押ししてコピーしてね",
    },
    steps: {
      share: {
        title: "共有ボタンをタップ",
        safari: "Safariのバーにあるよ。見つからないときは「…」の中をさがしてね。",
        browser: "アドレスバーの横にあるよ。",
      },
      add: { title: "「ホーム画面に追加」をえらぶ", body: "下にスクロールすると出てくるよ。" },
      confirm: { title: "右上の「追加」をタップ", body: "これでTokulyがホーム画面にならぶよ！" },
    },
    // Labels inside the little pictures of the steps, as iOS shows them.
    art: {
      copy: "コピー",
      addToHome: "ホーム画面に追加",
      bookmark: "ブックマークを追加",
      cancel: "キャンセル",
      add: "追加",
    },
    done: "わかった！",
  },

  status: {
    title: "いまの状態",
    install: {
      label: "インストール",
      yes: "済み",
      no: "まだ",
      yesHint: "この画面は、インストールしたアプリで開いてるよ。",
      noHint: "いまはブラウザで開いてるよ。",
    },
    push: {
      label: "プッシュ通知",
      on: "オン",
      off: "オフ",
      blocked: "ブロック中",
      unsupported: "非対応",
      onHint: "Tokulyを閉じていても、このデバイスにお知らせが届くよ。",
      offHint: "オンにすると、友達からのお知らせをすぐ受け取れるよ。",
      blockedHint: "ブラウザの設定で、このサイトの通知がブロックされてるよ。",
      unsupportedHint: "このブラウザはプッシュ通知に対応してないみたい。",
      needsInstallHint: "iPhone・iPadでは、ホーム画面に追加すると通知が使えるよ。",
      open: "通知設定をひらく",
    },
    offline: {
      label: "オフライン対応",
      ready: "準備OK",
      preparing: "準備中",
      unsupported: "非対応",
      readyHint: "電波が切れても専用の画面で待てて、つながったら自動でもどるよ。",
      preparingHint: "もうすこしで準備できるよ。",
      unsupportedHint: "このブラウザでは、電波がないときはブラウザのエラー画面になるよ。",
    },
    version: {
      label: "バージョン",
      latest: "最新",
      update: "更新あり",
      latestHint: "いちばん新しいTokulyを使ってるよ。",
      updateHint: "新しいバージョンがあるよ。更新すると画面を読み込みなおすよ。",
      check: "確認する",
      apply: "更新する",
      upToDate: "最新バージョンだよ！",
    },
  },

  storage: {
    title: "ストレージ",
    lead: "Tokulyがこのデバイスに保存しているデータの大きさだよ。",
    used: "使用中",
    of: "{{quota}} まで使えるよ",
    meter: "ストレージの使用量",
    unknown: "このブラウザでは、使用量がわからないみたい。",
    files: "すばやく開くために保存したファイル: {{count}}個",
    clear: "オフラインデータを消す",
    clearTitle: "オフラインデータを消す？",
    clearBody:
      "すばやく開くために保存したファイルを消して、容量を空けるよ。アカウントやメッセージは消えないから安心してね。次に開くときだけ、読み込みがすこし長くなるかも。",
    clearConfirm: "消す",
    cleared: "オフラインデータを消したよ！",
    clearFailed: "うまく消せなかった…もう一度ためしてね",
  },

  update: {
    toast: "新しいバージョンがあるよ！",
    reload: "更新する",
    later: "あとで",
  },

  offline: {
    label: "オフラインだよ",
  },
} as const;
