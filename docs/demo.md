# デモ（ボットの町）

はじめて開いたときに「友達がいるとこうなるんだ!」が分かるように、デモ用のボットを動かせます。
ボットは地図の上を動き、グループチャットでしゃべり、ノート・ストーリー・ショートを投稿します。ふだんはオフです。

```sh
tools/demo-env.sh        # .env にデモ用の設定を追記
docker compose up -d     # demo コンテナがボットを作って動かしはじめる
```

検索で `demo` と入れるとボット（`@demo_yui` など）が見つかります。友達リクエストを送るとすぐ承認され、
そのボットの友達からもリクエストが届きます。

| 変数（`.env`） | 既定 | 中身 |
|---|---|---|
| `COMPOSE_PROFILES=demo` | — | デモのコンテナを起動する |
| `TOKULY_DEV_DEMO_OPERATORS` | — | ボットの持ち主になるアカウントのメールアドレス（カンマ区切り、1 つにつきボット 5 体）。サーバーとシミュレーターの両方が読む |
| `TOKULY_DEV_DEMO_PASSWORD` | — | そのアカウントのパスワード（16 文字以上）。サーバーは、アドレスが一致してこのパスワードを知っているアカウントだけを持ち主として扱う |
| `TOKULY_DEMO_CENTER` / `TOKULY_DEMO_RADIUS_M` | `35.7030,139.5795` / `900` | ボットが動く中心（`緯度,経度`）と半径メートル。`TOKULY_MAP_REGION_BBOX` の中にする |
| `TOKULY_DEMO_BOTS` / `TOKULY_DEMO_SEED` / `TOKULY_DEMO_PACE` | `12` / `2026` / `1` | ボットの数、シード値、時間の進む速さ（1 = 実時間） |
| `TOKULY_DEMO_EPISODE` | 起動時刻 | 何が起きるか（ルート・投稿・会話）を決める値。固定すると毎回同じデモになる |

知っておくこと:

- ボットは公開 API だけを使うふつうのクライアントです。公開範囲・ブロック・レート制限はそのまま効きます。
- ボットには**ボットのしるし**が付き、位置は「シミュレーション」として送るので、Bump や「近くの人」には出ません。
- ボットから知らない人に話しかけることはありません。実在の人物や住所は出てきません。
- 持ち主に指定したアカウントのボットだけが、チャットや投稿などの書き込みをできます（コインは対象外）。
- 経路検索エンジン（`[geo] routing_url`）があれば実際の道に沿って歩きます。

やめるときは `COMPOSE_PROFILES` から `demo` を外して `docker compose up -d --remove-orphans`。
ボットのアカウントと投稿は残ります。消すときは持ち主のアカウントでログインして「開発者 → ボット」から削除します。

Docker なしでは、同じ変数を付けてサーバーを起動し、別のターミナルでシミュレーターを動かします:

```sh
tag=$(openssl rand -hex 6)    # 決まったアドレスだと先に登録されてしまうので毎回ランダムに
export TOKULY_DEV_DEMO_OPERATORS=demo-$tag-1@demo.tokuly.invalid,demo-$tag-2@demo.tokuly.invalid,demo-$tag-3@demo.tokuly.invalid
export TOKULY_DEV_DEMO_PASSWORD=$(openssl rand -hex 24)
(cd backend && cargo run -p tokuly-server -- all)
(cd backend && cargo run -p tokuly-server -- demo)      # --help で全オプション
```
