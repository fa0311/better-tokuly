/**
 * The Unicode emoji the picker offers, grouped the way people look for them,
 * each with the words it is found by in English and Japanese. The first
 * English word is its name. This module is loaded on demand (the picker and
 * the autocomplete import it lazily): it is data, not code.
 */
import type { UnicodeEmoji } from "@tokuly/core/emoji";

export type { UnicodeEmoji };

export type UnicodeGroupId =
  | "smileys"
  | "hearts"
  | "hands"
  | "animals"
  | "food"
  | "fun"
  | "symbols";

export interface UnicodeGroup {
  /** Key of the group's name in the `emoji` strings (`picker.groups.<id>`). */
  id: UnicodeGroupId;
  /** The emoji shown on the group's tab. */
  icon: string;
  emoji: UnicodeEmoji[];
}

/** One emoji per line: `emoji|english words|日本語のことば`. */
function parse(table: string): UnicodeEmoji[] {
  return table
    .trim()
    .split("\n")
    .map((line) => {
      const [emoji = "", en = "", ja = ""] = line.trim().split("|");
      return { emoji, en: en.split(" ").filter(Boolean), ja: ja.split(" ").filter(Boolean) };
    });
}

const SMILEYS = `
😀|grinning smile happy|にこにこ えがお 笑顔 うれしい
😃|smiley smile happy|にっこり えがお 笑顔
😄|smile happy laugh|えがお 笑顔 わらう 笑う
😁|grin beaming|にやにや にかっ えがお
😆|laughing lol haha|わらい 笑い ばくしょう 爆笑
🥹|holding_back_tears touched moved|うるうる かんどう 感動 なみだ
😅|sweat_smile phew|あせ 汗 ひやあせ 苦笑い
😂|joy lol tears|うれしなき 嬉し泣き わらい 爆笑 草
🤣|rofl rolling lol|だいばくしょう 大爆笑 わらい 草
🥲|smiling_tear|なきわらい 泣き笑い
😊|blush smile shy|てれ 照れ にこにこ うれしい
😇|innocent angel halo|てんし 天使 いいこ
🙂|slight_smile|ほほえみ 微笑み
🙃|upside_down|さかさま 逆さま
😉|wink|ウインク ういんく
😌|relieved calm|ほっ あんしん 安心
😍|heart_eyes love|めがハート 目がハート すき 好き
🥰|smiling_hearts love adore|だいすき 大好き ラブ しあわせ
😘|kiss kissing_heart|キス ちゅー チュー
😋|yum delicious tasty|おいしい 美味しい うまい ぺろり
😛|tongue|べー した 舌
😜|wink_tongue crazy|あっかんべー おちゃめ
🤪|zany crazy goofy|へんがお 変顔 ふざけ
😝|squint_tongue|べー あっかんべー
🤑|money_mouth rich|おかね お金 かねもち
🤗|hug hugging|ハグ はぐ ぎゅー
🤭|giggle hand_over_mouth oops|くすくす うふふ ぷぷ
🫢|gasp open_eyes_hand|はっ おどろき びっくり
🫣|peek peeking|ちらっ のぞき はずかしい
🤫|shush quiet secret|しー ないしょ 内緒 ひみつ
🤔|thinking hmm|かんがえる 考える うーん なやむ
🫡|salute yes_sir|けいれい 敬礼 りょうかい 了解
🤐|zipper_mouth|おくちチャック 口チャック ないしょ
🤨|raised_eyebrow suspicious|うたがう 疑う はて
😐|neutral|むひょうじょう 無表情 しんがお 真顔
😑|expressionless|まがお 真顔 しらー
😶|no_mouth silent|むごん 無言 だんまり
🫥|dotted_face invisible|きえたい 消えたい とうめい
😏|smirk|にやり どや ドヤ
😒|unamused meh|しらけ ふまん 不満 じとめ
🙄|eye_roll|あきれ 呆れ やれやれ
😬|grimace awkward|きまずい 気まずい いー
😮‍💨|exhale sigh|ためいき ため息 ふぅ
🤥|lying pinocchio|うそ 嘘 うそつき
😔|pensive sad|しょんぼり おちこむ 落ち込む
😪|sleepy|ねむい 眠い うとうと
🤤|drool|よだれ じゅるり
😴|sleeping zzz|ねる 寝る すやすや おやすみ
😷|mask sick|マスク かぜ 風邪
🤒|thermometer_face fever|ねつ 熱 かぜ 風邪
🤕|bandage hurt|けが 怪我 いたい 痛い
🤢|nauseated sick|きもちわるい 気持ち悪い はきけ
🤮|vomit|おえ はく 吐く
🥵|hot heat|あつい 暑い ほてり
🥶|cold freezing|さむい 寒い こごえる
🥴|woozy dizzy|ふらふら よっぱらい 酔っ払い
😵|dizzy_face|めがまわる 目が回る くらくら
😵‍💫|spiral_eyes dizzy|ぐるぐる めまい 混乱
🤯|mind_blown exploding_head|ばくはつ 爆発 しょうげき 衝撃
🤠|cowboy|カウボーイ
🥳|party celebrate|パーティー おいわい お祝い おめでとう
🥸|disguise|へんそう 変装
😎|sunglasses cool|サングラス クール かっこいい
🤓|nerd geek|オタク めがね メガネ がりべん
🧐|monocle inspect|かんさつ 観察 じーっ
😕|confused|こまった 困った はて
🫤|diagonal_mouth meh|びみょう 微妙
😟|worried|しんぱい 心配
🙁|frown|しゅん かなしい
😮|open_mouth wow|おどろき 驚き ぽかん わお
😯|hushed|びっくり おどろき
😲|astonished shocked|びっくり 驚き しょうげき
😳|flushed embarrassed|せきめん 赤面 はずかしい 恥ずかしい
🥺|pleading puppy_eyes|ぴえん おねがい お願い うるうる
😦|frown_open|がーん ショック
😧|anguished|くのう 苦悩
😨|fearful scared|こわい 怖い あおざめ
😰|anxious cold_sweat|ひやあせ 冷や汗 あせる 焦る
😥|sad_relieved sweat|がっかり あせ 汗
😢|cry sad tear|なく 泣く かなしい 悲しい なみだ
😭|sob crying loudly|ごうきゅう 号泣 なく 泣く ぴえん
😱|scream horror|ひめい 悲鳴 こわい ぎゃー
😖|confounded|こんわく 困惑 うー
😣|persevere|がまん 我慢 ぐぬぬ
😞|disappointed|がっかり ざんねん 残念
😓|downcast_sweat|ひやあせ つかれ 疲れ
😩|weary tired|へとへと つかれた 疲れた
😫|tired_face|つかれた 疲れた げんかい 限界
🥱|yawn bored|あくび ねむい たいくつ
😤|huff triumph|ふんっ ぷんぷん いきごみ
😡|rage pout angry|げきど 激怒 おこ 怒
😠|angry mad|おこる 怒る いかり 怒り
🤬|cursing swearing|ぶちぎれ 暴言
😈|smiling_imp devil|あくま 悪魔 こあくま
👿|imp angry_devil|あくま 悪魔
💀|skull dead|どくろ ドクロ がいこつ しんだ 死
👻|ghost|おばけ お化け ゆうれい 幽霊
👽|alien ufo|うちゅうじん 宇宙人 エイリアン
🤖|robot bot|ロボット ろぼっと
💩|poop|うんち うんこ
🙈|see_no_evil monkey|みざる 見ざる さる
🙉|hear_no_evil monkey|きかざる 聞かざる さる
🙊|speak_no_evil monkey|いわざる 言わざる さる
`;

const HEARTS = `
❤️|heart red_heart love|ハート はーと あい 愛 すき 好き
🩷|pink_heart|ピンク ハート
🧡|orange_heart|オレンジ ハート
💛|yellow_heart|きいろ 黄色 ハート
💚|green_heart|みどり 緑 ハート
🩵|light_blue_heart|みずいろ 水色 ハート
💙|blue_heart|あお 青 ハート
💜|purple_heart|むらさき 紫 ハート
🖤|black_heart|くろ 黒 ハート
🩶|grey_heart|グレー はいいろ ハート
🤍|white_heart|しろ 白 ハート
🤎|brown_heart|ちゃいろ 茶色 ハート
💔|broken_heart|しつれん 失恋 われたハート
❤️‍🔥|heart_on_fire|もえるハート 燃える じょうねつ 情熱
❤️‍🩹|mending_heart|いやし 癒し かいふく
❣️|heart_exclamation|ハート びっくり
💕|two_hearts|ハート ふたつ らぶらぶ ラブラブ
💞|revolving_hearts|ハート くるくる
💓|heartbeat|どきどき ドキドキ こどう
💗|growing_heart|ときめき きゅん
💖|sparkling_heart|きらきらハート ときめき
💘|cupid heart_arrow|キューピッド こい 恋
💝|gift_heart|プレゼント ハート おくりもの
💟|heart_decoration|ハート かざり
💌|love_letter|ラブレター てがみ 手紙
💋|kiss_mark lips|キスマーク くちびる 唇
💯|100 hundred perfect|ひゃくてん 百点 まんてん 満点
💢|anger|いかり 怒り おこ ぷんぷん
💥|boom collision|ばくはつ 爆発 どーん
💫|dizzy star|くらくら ほし 星
💦|sweat_drops|あせ 汗 しずく
💨|dash wind|ダッシュ かぜ 風 ぴゅー
💤|zzz sleep|ねむい 眠い ぐーぐー すやすや
`;

const HANDS = `
👍|thumbsup +1 like good yes|いいね グッド ぐっど さんせい 賛成
👎|thumbsdown -1 bad no|よくないね バッド はんたい 反対
👏|clap applause|はくしゅ 拍手 ぱちぱち
🙌|raised_hands hooray|ばんざい 万歳 やったー
🫶|heart_hands|ゆびハート てでハート 手でハート
🤝|handshake deal|あくしゅ 握手
🙏|pray please thanks|おねがい お願い ありがとう かんしゃ 感謝 いのり
👌|ok_hand|オーケー おっけー OK
🤌|pinched_fingers|なんで イタリア
🤏|pinch small|ちょっと すこし 少し
✌️|v peace victory|ピース ぴーす ブイ
🤞|crossed_fingers luck|いのる 祈る こううん 幸運
🫰|finger_heart|ゆびハート 指ハート
🤟|love_you_gesture|あいしてる 愛してる
🤘|rock_on metal|ロック メタル
🤙|call_me shaka|でんわして アロハ
👈|point_left|ひだり 左 ゆびさし
👉|point_right|みぎ 右 ゆびさし
👆|point_up_2|うえ 上 ゆびさし
👇|point_down|した 下 ゆびさし
☝️|point_up one|ひとつ いち これ
✋|raised_hand stop high_five|て 手 ストップ ハイタッチ
🖐️|hand_splayed five|パー ぱー て 手
👋|wave hello bye|バイバイ ばいばい やっほー こんにちは てをふる
🤚|raised_back_of_hand|てのこう 手の甲
💪|muscle strong flex|きんにく 筋肉 がんばる 頑張る ちから
🫵|point_at_you|あなた きみ 君
✊|fist|グー ぐー こぶし
👊|punch fist_bump|パンチ ぱんち グータッチ
🤛|left_fist|グータッチ ひだり
🤜|right_fist|グータッチ みぎ
✍️|writing|かく 書く メモ
💅|nail_polish|ネイル マニキュア
🤳|selfie|じどり 自撮り セルフィー
👀|eyes look|め 目 みる 見る じー
👁️|eye|め 目
🧠|brain smart|のう 脳 あたま 頭
🫀|anatomical_heart|しんぞう 心臓
👑|crown king queen|おうかん 王冠 キング クイーン
`;

const ANIMALS = `
🐶|dog puppy|いぬ 犬 わんこ わんわん
🐱|cat kitty|ねこ 猫 にゃんこ にゃー
🐭|mouse|ねずみ 鼠 ちゅー
🐹|hamster|ハムスター はむ
🐰|rabbit bunny|うさぎ 兎 うさ
🦊|fox|きつね 狐
🐻|bear|くま 熊
🐼|panda|パンダ ぱんだ
🐻‍❄️|polar_bear|しろくま 白熊
🐨|koala|コアラ こあら
🐯|tiger|とら 虎
🦁|lion|ライオン らいおん
🐮|cow|うし 牛
🐷|pig|ぶた 豚
🐸|frog|かえる 蛙 けろ
🐵|monkey_face|さる 猿
🐔|chicken|にわとり 鶏 こけこっこ
🐧|penguin|ペンギン ぺんぎん
🐦|bird|とり 鳥 ことり
🐤|chick|ひよこ ぴよ
🦆|duck|あひる かも 鴨
🦉|owl|ふくろう 梟
🦄|unicorn|ユニコーン ゆにこーん
🐝|bee|はち 蜂 みつばち
🦋|butterfly|ちょう 蝶 ちょうちょ
🐌|snail|かたつむり でんでんむし
🐞|ladybug|てんとうむし
🐢|turtle|かめ 亀
🐍|snake|へび 蛇
🐙|octopus|たこ 蛸
🦀|crab|かに 蟹
🐠|tropical_fish fish|さかな 魚 ねったいぎょ
🐬|dolphin|いるか イルカ
🐳|whale|くじら 鯨
🦈|shark|さめ 鮫
🐘|elephant|ぞう 象
🦒|giraffe|きりん キリン
🦥|sloth|なまけもの
🦦|otter|かわうそ ラッコ
🐾|paw_prints|あしあと 足跡 にくきゅう 肉球
🌸|cherry_blossom sakura|さくら 桜 はな 花
🌷|tulip|チューリップ はな 花
🌹|rose|ばら 薔薇
🌻|sunflower|ひまわり 向日葵
🌼|blossom|はな 花 おはな
🍀|four_leaf_clover luck|クローバー よつば 四つ葉 こううん
🌱|seedling sprout|め 芽 ふたば 双葉 草
🌵|cactus|サボテン さぼてん
🌴|palm_tree|やし ヤシ なんごく 南国
🍁|maple_leaf autumn|もみじ 紅葉 あき 秋
🍂|fallen_leaf|おちば 落ち葉 あき 秋
🌙|moon crescent|つき 月 みかづき 三日月 よる 夜
⭐|star|ほし 星 スター
🌟|glowing_star|ほし 星 きらきら かがやき
☀️|sun sunny|たいよう 太陽 はれ 晴れ
🌈|rainbow|にじ 虹
☁️|cloud cloudy|くも 雲 くもり 曇り
⚡|zap lightning thunder|かみなり 雷 でんき 電気
❄️|snowflake snow|ゆき 雪 けっしょう
🔥|fire flame hot lit|ほのお 炎 ひ 火 あつい もえる 燃える
💧|droplet water|しずく 雫 みず 水
🌊|wave ocean sea|なみ 波 うみ 海
`;

const FOOD = `
🍙|rice_ball onigiri|おにぎり おむすび
🍘|rice_cracker senbei|せんべい 煎餅
🍣|sushi|すし 寿司 おすし
🍱|bento|べんとう 弁当 おべんとう
🍜|ramen noodles|ラーメン らーめん めん 麺
🍛|curry|カレー かれー
🍚|rice|ごはん ご飯 こめ 米
🍡|dango|だんご 団子
🍢|oden|おでん
🥟|dumpling gyoza|ぎょうざ 餃子
🍤|fried_shrimp tempura|えびフライ てんぷら 天ぷら
🍥|fish_cake naruto|なると ナルト
🍕|pizza|ピザ ぴざ
🍔|hamburger burger|ハンバーガー バーガー
🍟|fries|ポテト ぽてと フライドポテト
🌭|hotdog|ホットドッグ
🌮|taco|タコス
🥪|sandwich|サンドイッチ サンド
🍝|spaghetti pasta|パスタ スパゲッティ
🥗|salad|サラダ さらだ
🍳|fried_egg cooking|めだまやき 目玉焼き たまご 卵 りょうり 料理
🧇|waffle|ワッフル
🥞|pancakes|パンケーキ ホットケーキ
🍞|bread|パン しょくぱん 食パン
🥐|croissant|クロワッサン
🧀|cheese|チーズ ちーず
🍗|poultry_leg chicken|チキン からあげ 唐揚げ にく 肉
🍖|meat_on_bone|にく 肉 まんがにく
🍰|cake shortcake|ケーキ けーき ショートケーキ
🎂|birthday_cake|たんじょうび 誕生日 バースデー ケーキ
🧁|cupcake|カップケーキ
🍮|pudding custard|プリン ぷりん
🍩|doughnut donut|ドーナツ どーなつ
🍪|cookie|クッキー くっきー
🍫|chocolate|チョコ チョコレート
🍬|candy|あめ 飴 キャンディ
🍭|lollipop|ぺろぺろキャンディ あめ 飴
🍦|soft_serve icecream|ソフトクリーム アイス
🍨|ice_cream|アイス アイスクリーム
🍧|shaved_ice|かきごおり かき氷
🍎|apple|りんご 林檎
🍊|tangerine orange|みかん オレンジ
🍋|lemon|レモン れもん
🍌|banana|バナナ ばなな
🍉|watermelon|すいか 西瓜
🍇|grapes|ぶどう 葡萄
🍓|strawberry|いちご 苺
🍑|peach|もも 桃
🍒|cherries|さくらんぼ
🥝|kiwi|キウイ きうい
🍵|tea green_tea matcha|おちゃ お茶 りょくちゃ 緑茶 まっちゃ 抹茶
☕|coffee|コーヒー こーひー カフェ
🧋|bubble_tea boba|タピオカ たぴおか
🥤|cup_with_straw soda|ジュース ドリンク のみもの 飲み物
🧃|juice_box|ジュース かみパック
🍺|beer|ビール びーる
🍻|beers cheers|かんぱい 乾杯 ビール
🥂|champagne_glasses toast|かんぱい 乾杯 シャンパン
`;

const FUN = `
🎉|tada party celebrate|おめでとう おいわい お祝い クラッカー パーティー
🎊|confetti_ball|くすだま くす玉 おいわい
🎈|balloon|ふうせん 風船
🎁|gift present|プレゼント ぷれぜんと おくりもの 贈り物
🎀|ribbon|リボン りぼん
🏆|trophy win|トロフィー ゆうしょう 優勝
🥇|first_place gold_medal|きんメダル 金メダル いちい 一位
🎯|dart bullseye target|まと 的 めいちゅう 命中 もくひょう
🎮|video_game controller gaming|ゲーム げーむ コントローラー
🕹️|joystick|ジョイスティック ゲーム
🎲|die dice|サイコロ さいころ
🧩|puzzle|パズル ぱずる
🎨|art palette|え 絵 アート パレット おえかき
🎬|clapper movie|えいが 映画 カチンコ どうが 動画
🎤|microphone karaoke sing|マイク カラオケ うた 歌
🎧|headphones|ヘッドホン おんがく 音楽
🎵|musical_note|おんぷ 音符 おんがく 音楽
🎶|notes music|おんぷ 音符 おんがく 音楽 るんるん
🎸|guitar|ギター ぎたー
🎹|piano keyboard|ピアノ ぴあの けんばん
🥁|drum|ドラム たいこ 太鼓
📸|camera_flash photo|カメラ しゃしん 写真 さつえい
📱|phone smartphone|スマホ けいたい 携帯
💻|laptop computer pc|パソコン ピーシー ノートパソコン
📚|books study|ほん 本 べんきょう 勉強
✏️|pencil|えんぴつ 鉛筆
📝|memo note|メモ めも ノート
🎓|graduation_cap|そつぎょう 卒業
🏫|school|がっこう 学校
⚽|soccer football|サッカー さっかー
🏀|basketball|バスケ バスケットボール
⚾|baseball|やきゅう 野球
🎾|tennis|テニス てにす
🏐|volleyball|バレー バレーボール
🏓|ping_pong table_tennis|たっきゅう 卓球
🏸|badminton|バドミントン
🏃|runner running|はしる 走る ランニング ダッシュ
🚴|cyclist bike|じてんしゃ 自転車 サイクリング
🏊|swimmer swim|すいえい 水泳 およぐ 泳ぐ
⛷️|skier|スキー すきー
🎿|ski|スキー スキーいた
🛹|skateboard|スケボー スケートボード
🚃|train railway|でんしゃ 電車
🚗|car|くるま 車 ドライブ
✈️|airplane flight|ひこうき 飛行機 りょこう 旅行
🚀|rocket launch|ロケット ろけっと はっしゃ
🗼|tokyo_tower|とうきょうタワー 東京タワー
🗾|japan map|にほん 日本 ちず 地図
⛩️|shinto_shrine torii|じんじゃ 神社 とりい 鳥居
🏯|japanese_castle|しろ 城 おしろ
🎡|ferris_wheel|かんらんしゃ 観覧車
🎢|roller_coaster|ジェットコースター
🎆|fireworks|はなび 花火
🎇|sparkler|せんこうはなび 線香花火 はなび
🏝️|island beach|しま 島 ビーチ なんごく
⛰️|mountain|やま 山
🏕️|camping tent|キャンプ きゃんぷ テント
💸|money_wings|おかね お金 しゅっぴ 出費 きんけつ 金欠
💰|moneybag|おかね お金 かねぶくろ
💎|gem diamond|ほうせき 宝石 ダイヤ
`;

const SYMBOLS = `
✨|sparkles shiny|きらきら キラキラ かがやき
💬|speech_balloon comment|ふきだし 吹き出し コメント
💭|thought_balloon|かんがえ 考え もうそう
🗯️|anger_bubble|いかり ふきだし
✅|check done ok|チェック かんりょう 完了 オッケー
❌|x cross no wrong|ばつ バツ だめ ダメ まちがい
⭕|o circle correct|まる マル せいかい 正解
❗|exclamation|びっくり ビックリ
❓|question|はてな ハテナ しつもん 質問
‼️|bangbang|びっくり にじゅう
⁉️|interrobang|びっくりはてな まじ
🔔|bell notification|ベル つうち 通知 かね
🔕|no_bell mute|ミュート つうちオフ
📣|megaphone cheer|メガホン おうえん 応援
📢|loudspeaker announce|おしらせ お知らせ スピーカー
🆗|ok|オーケー おっけー
🆒|cool|クール
🆕|new|しん 新 ニュー あたらしい
🆙|up|アップ
🈵|full|まん 満 まんいん
🈳|vacancy empty|くう 空 あき
🉐|bargain|とく 得 おとく お得
㊗️|congratulations|いわい 祝 おめでとう
㊙️|secret|ひみつ 秘密 まるひ
🔞|underage adult|じゅうはちきん 18禁
🚫|prohibited no|きんし 禁止 だめ
⚠️|warning caution|けいこく 警告 ちゅうい 注意
♻️|recycle|リサイクル
🔴|red_circle|あか 赤 まる
🟠|orange_circle|オレンジ まる
🟡|yellow_circle|きいろ 黄色 まる
🟢|green_circle|みどり 緑 まる
🔵|blue_circle|あお 青 まる
🟣|purple_circle|むらさき 紫 まる
⚫|black_circle|くろ 黒 まる
⚪|white_circle|しろ 白 まる
🔺|red_triangle up|さんかく 三角 うえ
🔻|red_triangle_down|さんかく 三角 した
▶️|play|さいせい 再生
⏸️|pause|いちじていし 一時停止
⏹️|stop|ていし 停止
🔁|repeat loop|リピート くりかえし
🔀|shuffle|シャッフル
➕|plus add|プラス たす 足す
➖|minus|マイナス ひく 引く
✖️|multiply|かける 掛ける ばつ
➗|divide|わる 割る
♾️|infinity|むげん 無限
🏳️‍🌈|rainbow_flag pride|レインボー にじ 虹 はた
🇯🇵|jp japan flag|にほん 日本 こっき 国旗
`;

export const UNICODE_GROUPS: UnicodeGroup[] = [
  { id: "smileys", icon: "😀", emoji: parse(SMILEYS) },
  { id: "hearts", icon: "❤️", emoji: parse(HEARTS) },
  { id: "hands", icon: "👍", emoji: parse(HANDS) },
  { id: "animals", icon: "🐱", emoji: parse(ANIMALS) },
  { id: "food", icon: "🍙", emoji: parse(FOOD) },
  { id: "fun", icon: "🎉", emoji: parse(FUN) },
  { id: "symbols", icon: "✨", emoji: parse(SYMBOLS) },
];

/** Every Unicode emoji of the picker, in group order. */
export const UNICODE_EMOJI: UnicodeEmoji[] = UNICODE_GROUPS.flatMap((group) => group.emoji);
