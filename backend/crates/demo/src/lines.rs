//! What the bots say: every line of the demo, in one place.
//!
//! Casual Japanese the way high schoolers write it. Nobody here is a real
//! person and no place is a real address: the names are the kind every
//! town has ("駅前", "いつもの公園").

/// A name a bot goes by: how it is written, and the handle it becomes.
pub struct Name {
    pub kana: &'static str,
    pub romaji: &'static str,
}

const fn name(kana: &'static str, romaji: &'static str) -> Name {
    Name { kana, romaji }
}

pub const NAMES: &[Name] = &[
    name("ゆい", "yui"),
    name("はると", "haruto"),
    name("みお", "mio"),
    name("そうた", "sota"),
    name("ひな", "hina"),
    name("りく", "riku"),
    name("こはる", "koharu"),
    name("れん", "ren"),
    name("あおい", "aoi"),
    name("かいと", "kaito"),
    name("めい", "mei"),
    name("ゆうま", "yuma"),
    name("さくら", "sakura"),
    name("そら", "sora"),
    name("りこ", "riko"),
    name("みなと", "minato"),
    name("つむぎ", "tsumugi"),
    name("いつき", "itsuki"),
    name("ほのか", "honoka"),
    name("だいち", "daichi"),
    name("ののか", "nonoka"),
    name("あさひ", "asahi"),
    name("すず", "suzu"),
    name("たくみ", "takumi"),
];

/// Decorations after a display name.
pub const NAME_MARKS: &[&str] = &["", "", "🍙", "🎧", "🌸", "⚡", "🐈", "🍀", "🫧", "🏀", "🎸", "🌙"];

pub const GRADES: &[&str] = &["高1", "高2", "高3"];

pub const CLUBS: &[&str] = &[
    "バスケ部",
    "軽音部",
    "帰宅部のエース",
    "吹奏楽部",
    "ダンス部",
    "美術部",
    "陸上部",
    "写真部",
    "放送部",
    "テニス部",
    "サッカー部",
    "演劇部",
];

pub const LIKES: &[&str] = &[
    "たい焼き",
    "カラオケ",
    "ガチャガチャ",
    "深夜ラジオ",
    "ねこ",
    "プリ",
    "アニメ",
    "タピオカ",
    "古着",
    "音ゲー",
    "コンビニの新作スイーツ",
    "散歩",
];

/// Names of the group chats.
pub const GROUP_NAMES: &[&str] =
    &["いつメン🍙", "放課後組🎒", "テスト勉強会📚", "カラオケ行く人〜🎤", "帰り道同盟🚲", "ひまじん集合🫠"];

/// Somewhere the bots go.
pub struct Spot {
    pub name: &'static str,
    /// The status a bot shows while it is there.
    pub emoji: &'static str,
    pub status: &'static str,
    /// Notes posted from there.
    pub notes: &'static [&'static str],
}

pub const SPOTS: &[Spot] = &[
    Spot {
        name: "駅前",
        emoji: "🚉",
        status: "駅前でまちあわせ",
        notes: &["駅前ついた！だれかいる〜？", "駅前のベンチ、今日もうまってる笑"],
    },
    Spot {
        name: "学校",
        emoji: "🏫",
        status: "がっこう",
        notes: &["やっと6限おわった〜〜〜", "部活いってきます！！", "教室にスマホの充電器わすれた…"],
    },
    Spot {
        name: "いつもの公園",
        emoji: "🌳",
        status: "公園でまったり",
        notes: &["公園のブランコ、ひさびさに乗ったら酔った", "夕方の公園、風きもちいい🍃"],
    },
    Spot {
        name: "駅前のカフェ",
        emoji: "☕",
        status: "カフェで勉強中",
        notes: &["カフェで勉強するって言って30分しゃべってる", "新作のフラペ、あますぎて最高だった"],
    },
    Spot {
        name: "コンビニ",
        emoji: "🍙",
        status: "コンビニ寄り道",
        notes: &["コンビニの新作スイーツ、全部おいしそうで決められん", "肉まんの季節きた！！！"],
    },
    Spot {
        name: "図書館",
        emoji: "📚",
        status: "図書館で自習",
        notes: &["図書館しずかすぎて自分のおなかの音がひびく", "テスト前だけ図書館の民になる"],
    },
    Spot {
        name: "ゲーセン",
        emoji: "🕹️",
        status: "ゲーセンなう",
        notes: &["UFOキャッチャーに500円すいこまれた", "音ゲー自己ベスト更新した！！ほめて！"],
    },
    Spot {
        name: "カラオケ",
        emoji: "🎤",
        status: "カラオケなう",
        notes: &["カラオケ3時間コース、のど終了のお知らせ", "採点で92点でた🎤 天才かもしれん"],
    },
    Spot {
        name: "ファミレス",
        emoji: "🍟",
        status: "ファミレスでだらだら",
        notes: &["ドリンクバーで3時間ねばってる", "ポテト山盛りたのんだら一瞬できえた"],
    },
    Spot {
        name: "本屋",
        emoji: "📖",
        status: "本屋で立ち読み",
        notes: &["マンガの新刊でてた！買った！", "参考書コーナー見るだけ見て帰る"],
    },
    Spot {
        name: "川ぞいの道",
        emoji: "🌇",
        status: "川ぞい散歩中",
        notes: &["川ぞいの夕焼けがきれいすぎた", "散歩してたら2駅ぶん歩いてた"],
    },
    Spot {
        name: "体育館",
        emoji: "🏀",
        status: "体育館で練習",
        notes: &["今日の練習きつすぎ、もう足がうごかん", "シュート10本連続で入った🏀"],
    },
    Spot {
        name: "たい焼き屋",
        emoji: "🐟",
        status: "たい焼き待ち",
        notes: &["たい焼き、あんこ派 vs カスタード派で戦争になってる", "焼きたてのたい焼きしか勝たん"],
    },
    Spot {
        name: "クレープ屋",
        emoji: "🍓",
        status: "クレープたべてる",
        notes: &["クレープ、いちごチョコ生クリームがいちばんつよい", "クレープ落とした…………"],
    },
];

/// Notes posted from anywhere.
pub const NOTES: &[&str] = &[
    "ねむすぎて1限の記憶がない",
    "明日の小テスト、範囲どこだっけ…だれか教えて🙏",
    "今日の空めっちゃきれい",
    "体育のあとの数学は無理ゲー",
    "お弁当わすれて購買ダッシュした",
    "席替えで窓ぎわゲットした！！かった！",
    "推しの新曲でた！！！ずっと聴いてる",
    "前髪きりすぎた。だれにも会いたくない",
    "雨の日の自転車、ほぼ修行",
    "課題おわらん〜〜〜だれか一緒にやろ",
    "週末ひまな人いる？どっか行きたい",
    "英単語、100こ覚えて90こ忘れた",
    "放課後のアイスがいちばんうまい説",
    "今日なんか全部うまくいく日だった",
    "文化祭の準備、うちのクラスだけ進んでなくて草",
    "帰り道でねこに会えた🐈 いい日",
    "スマホの充電3%で生きてる",
    "テストおわったら全力で遊ぶって決めてる",
    "夜ふかしやめるって言って今日も2時",
    "体育祭の練習で日焼けした…",
    "みんなの位置みてたら全員ばらばらで笑う",
    "今日のお昼、からあげ争奪戦に勝った",
    "プリとったら盛れすぎて別人",
    "あしたから本気だす（3回目）",
    "イヤホンかたっぽなくした…だれか見てない？",
    "朝の電車で座れた！それだけでうれしい",
    "古典の先生の雑談がいちばんおもしろい",
    "月曜なのにもう金曜の気分",
    "新しいスニーカーおろした👟",
    "ひまだからだれかマップで見つけて〜",
];

/// Answers to somebody's note.
pub const REPLIES: &[&str] = &[
    "わかる〜〜",
    "それな",
    "えっいいな！",
    "草",
    "天才か？",
    "こんど一緒にいこ！",
    "おつかれ〜！",
    "ずるい！私も！",
    "まじか笑",
    "えらすぎる",
];

/// Captions of stories.
pub const STORY_CAPTIONS: &[&str] = &[
    "きょうの空🌇",
    "放課後〜",
    "ひさびさに集合！",
    "テストおわった！！！",
    "寄り道ちゅう",
    "今日のごほうび🍦",
    "部活おつかれ",
    "ねむい",
    "いい天気すぎる",
    "帰り道",
];

/// Captions of shorts.
pub const SHORT_CAPTIONS: &[&str] = &[
    "放課後のテンション #放課後 #デモ",
    "なんか撮れた笑 #日常 #デモ",
    "きょうの帰り道 #帰り道 #デモ",
    "テストおわりの解放感 #テスト明け #デモ",
    "ひまだったので #ひまつぶし #デモ",
    "色がきれいだった #きれい #デモ",
];

/// Reactions to stories and shorts.
pub const REACTIONS: &[&str] = &["😂", "🔥", "🥺", "👏", "😍", "🎉", "👀", "💯"];

/// Conversations, line by line; the speakers take turns.
pub const DIALOGUES: &[&[&str]] = &[
    &["今どこ〜？", "駅前！", "おけ、5分でいく🏃", "まってる〜"],
    &["数学の課題おわった？", "まだ1ミリもやってない", "仲間いて安心した笑", "明日の朝いっしょにやろ…"],
    &["今日カラオケいかん？", "いく！！", "何時から？", "5時に駅前で！", "りょ🎤"],
    &["マップ見たら近くにいるじゃん", "ほんとだ笑", "そっち行くわ", "コンビニの前にいるよ〜"],
    &["お昼なに食べた？", "からあげ弁当", "いいな〜こっちパン1こ", "それは足りんて"],
    &[
        "テスト範囲ひろすぎん？",
        "それな、もうあきらめた",
        "あきらめるの早い笑",
        "いっしょに勉強しよ📚",
        "図書館あつまる？",
    ],
    &["ねえ聞いて", "なになに", "席替えで窓ぎわになった！", "勝ち組じゃん"],
    &["雨ふってきた☔", "かさ持ってない…", "いれてあげよっか", "神！！"],
    &["土曜ひま？", "ひま！", "買い物つきあって〜", "いいよ、何時？", "11時に駅前で！"],
    &["新作のフラペのんだ？", "まだ！おいしい？", "あますぎて最高", "明日いこ"],
    &["部活おわった〜", "おつかれ！", "アイスたべたい", "コンビニ寄ろ🍦"],
    &["充電3%しかない", "また？笑", "モバ充かして🙏", "しゃーないな〜"],
    &["今日の英語あてられた…", "見てた笑", "助けてよ！", "ごめん寝てた"],
    &["プリとりにいこ", "いく！盛れるやつがいい", "新しい機種はいったらしい", "それはいくしかない"],
    &["帰りどっち方面？", "川ぞい通ってかえる", "いっしょに帰ろ〜", "おけ、校門でまってる"],
    &["明日なんか持ち物あったっけ", "体操服！", "あぶな、忘れてた", "あと古典のプリントも"],
    &["もう着いた？", "あと2分！", "マップで見えてるよ笑", "監視されてる👀"],
    &["文化祭なにやる？", "おばけ屋敷がいい", "準備たいへんそう", "でもぜったい楽しい！"],
];

/// What a bot writes to someone who just became its friend.
pub const GREETINGS: &[&str] = &[
    "友達追加ありがと〜！デモ用のボットだけど、マップで動いてるから見ててね👀",
    "よろしく〜！ボットだけどふつうに話しかけてね🤖",
    "追加ありがとう！今日もそのへん歩いてるよ🚶",
];

/// The greeting on a friend request a bot sends to a visitor.
pub const INTRODUCTIONS: &[&str] =
    &["{friend}の友達だよ！デモのボットです🤖 よろしく〜", "{friend}から聞いた！ボットだけど友達になろ〜🤖"];

/// Answers to whatever a visitor writes.
pub const ANSWERS: &[&str] = &[
    "メッセージありがと！デモのボットだから決まったことしか言えないんだ🤖 ごめんね",
    "おお〜！（ボットなので気のきいた返事はできません笑）",
    "わかる〜！…って言っておくね🤖",
    "いまそのへん歩いてるから、マップで見つけてみて👀",
    "ノートとストーリーも投稿してるから見てね〜📝",
];
