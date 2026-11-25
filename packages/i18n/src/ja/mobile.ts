/** The screens only the phone app has: tabs, location permission, background sharing, settings. */
export default {
  tabs: { map: "マップ", friends: "ともだち", me: "じぶん" },
  welcome: {
    signUp: "はじめる",
    signIn: "ログイン",
  },
  accounts: {
    continueTitle: "この端末のアカウント",
    continue: "@{{handle}}で続ける",
    signInAnother: "ほかのアカウントでログイン",
  },
  sharing: {
    explain: {
      title: "いまどこ？をシェアしよう！",
      body: "オンにすると、ともだちのマップにあなたが出るよ。ゴーストモードでぼかしたり止めたりもできるし、いつでもやめられるよ。",
      whileUsing: "まずは「アプリの使用中は許可」を選んでね",
      always: "アプリを閉じている間も届けたいなら、そのあと「常に許可」に！",
    },
    turnOn: "オンにする",
    notNow: "あとで",
    sharing: {
      title: "いまを共有中",
      foreground: "アプリを開いている間、ともだちにいまの場所が見えるよ。",
      background: "アプリを閉じていても、ともだちにいまが届くよ。",
    },
    alwaysAsk: "閉じている間も届ける",
    alwaysHint: "次の画面で「常に許可」を選んでね",
    approximate:
      "おおよその位置だけ許可されてるよ。「正確な位置情報」をオンにすると、ともだちにちゃんと届くよ。",
    blocked: {
      title: "位置情報がオフになってるよ",
      body: "設定アプリで Tokuly の位置情報を「使用中は許可」か「常に許可」にしてね。",
    },
    servicesOff: {
      title: "端末の位置情報サービスがオフだよ",
      body: "コントロールセンターや設定から位置情報サービスをオンにしてね。",
    },
    openSettings: "設定をひらく",
    stopped: { title: "共有をお休み中", body: "ともだちには最後の場所のまま見えてるよ。" },
    resume: "再開する",
    stop: "共有をやめる",
    notification: {
      title: "いまを共有中",
      body: "ともだちのマップにあなたのいまを届けてるよ",
    },
  },
  map: {
    ghost: "ゴースト",
    sharingOff: "共有オフ",
    friendsOnMap: "マップに{{count}}人",
  },
  friends: {
    add: "ともだちを追加",
    addTitle: "ともだちを追加",
    byHandle: "ハンドルで追加",
    handle: "ハンドル",
    handlePlaceholder: "例: tokuly_chan",
    send: "リクエストを送る",
    byInvite: "招待リンクで追加",
    inviteLink: "招待リンク または コード",
    invitePlaceholder: "https://…/invite/xxxx",
    open: "ひらく",
    myInvite: "あなたの招待リンク",
    shareInvite: "招待リンクをシェア",
    shareMessage: "Tokulyでともだちになろう！ {{url}}",
    invitedBy: "{{name}}さんからの招待！",
    inviteLead: "ともだちになると、おたがいのいまがマップで見えるよ。",
    alreadyFriends: "もうともだちだよ！",
    invalidInvite: "この招待リンクは使えないみたい",
  },
  settings: {
    title: "設定",
    accounts: "アカウント切り替え",
    web: "そのほかの設定",
    webHint: "まだアプリにない設定は、Webでひらくよ",
    webNote: "Webでもう一度ログインしてね。アプリのログイン情報は、安全のためWebに渡さないよ。",
    signOut: "ログアウト",
    signOutConfirm:
      "このアカウントをこの端末からログアウトしますか？ほかのアカウントはそのままです。",
    version: "バージョン {{version}}",
    server: "サーバー {{url}}",
  },
  profile: {
    me: "じぶん",
    viewOnWeb: "Webでひらく",
  },
} as const;
