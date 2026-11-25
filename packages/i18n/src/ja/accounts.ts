export default {
  tile: "アカウント切り替え",
  title: "アカウント切り替え",
  listTitle: "この端末のアカウント",
  lead: "メイン・裏垢・クリエイター用…この端末で使うアカウントをワンタップで切り替えよう！",
  active: "使用中",
  switch: "切り替え",
  switched: "{{name}}に切り替えました！",
  remove: "@{{handle}}をこの端末からログアウト",
  removeConfirm: "@{{handle}}をこの端末からログアウトしますか？ほかのアカウントはそのままです。",
  removed: "この端末からログアウトしました",
  needsCode: "切り替えに2段階認証のコードが必要",
  add: "アカウントを追加",
  create: "新しいアカウントを作る",
  full: "この端末に入れられるのは{{count}}アカウントまでです",
  privacyNote:
    "アカウント同士はつながりません。ほかの人にも、それぞれのアカウントにも、同じ人だとはわかりません。",
  codeTitle: "@{{handle}}の確認コード",
  codeLead:
    "このアカウントは2段階認証がオンです。認証アプリのコード（またはリカバリーコード）を入れてね。",
  code: "確認コード",
  codeSubmit: "切り替える",
  onDevice: "この端末のアカウントで続ける",
  addTitle: "アカウントを追加",
  addLead: "いまのアカウントはそのまま。あとから切り替えられます！",
  errors: {
    ACCOUNT_NOT_ON_DEVICE:
      "このアカウントはこの端末からログアウトしています。もう一度ログインしてね",
    ACCOUNT_ALREADY_ACTIVE: "このアカウントはもう使用中です",
    TOO_MANY_ACCOUNTS: "この端末にはこれ以上アカウントを追加できません",
    SWITCH_REQUIRED: "アカウントを切り替えてね",
  },
} as const;
