# メモリ

`docker compose` の各サービスにはメモリの上限（`mem_limit`）があり、超えたサービスだけが落ちて再起動します。
マシン全体が足りなくなったときも、カーネルは PostgreSQL を最後に選びます（`oom_score_adj`）。
既定は **4 GB** のマシン向けです。**8 GB** 以上なら、下の行を `.env` に足して `docker compose up -d` します。

```sh
POSTGRES_SHARED_BUFFERS=512MB
POSTGRES_WORK_MEM=8MB
POSTGRES_EFFECTIVE_CACHE_SIZE=3GB
POSTGRES_MEM_LIMIT=1536m
POSTGRES_MEM_RESERVATION=1g
CLICKHOUSE_MEM_LIMIT=1536m
MEILI_MAX_INDEXING_MEMORY=384MiB
MEILI_MAX_INDEXING_THREADS=2
SEARCH_MEM_LIMIT=768m
VALKEY_MAXMEMORY=384mb
VALKEY_MEM_LIMIT=512m
WORKER_MEM_LIMIT=1536m
```

| サービス | 4 GB | 8 GB | 実測（待機 / デモ中の最大） | 自分で守る上限 |
|---|---|---|---|---|
| postgres | 768 MB | 1.5 GB | 150 / 370 MB | `shared_buffers` 256 MB・`work_mem` 4 MB（8 GB: 512 MB・8 MB）、`max_connections=100` |
| clickhouse | 768 MB | 1.5 GB | 420 / 460 MB | 上限の 3/4、キャッシュは数十 MB、診断用のログ表はなし（[`infra/clickhouse/`](../infra/clickhouse/)） |
| search（Meilisearch） | 384 MB | 768 MB | 190 / 230 MB | 索引づくりに 128 MiB・1 スレッド（8 GB: 384 MiB・2） |
| valkey | 256 MB | 512 MB | 10 / 10 MB | `maxmemory` 128 MB（8 GB: 384 MB） |
| worker | 768 MB | 1.5 GB | 130 / 290 MB | FFmpeg が動画を変換する分 |
| api / realtime | 384 / 256 MB | 同じ | 114 / 152・89 / 92 MB | |
| その他 10 個 | 各 128〜384 MB | 同じ | 合計 300 / 360 MB | |
| **合計** | | | **1.4 / 2.0 GB** | |

実測は 4 GB の VM で、デモ（ボット 12 体）を動かしながら `tools/ci/smoke.sh` が通ったときのものです
（`docker stats`、5 秒ごと）。上限の合計はマシンの RAM より大きく、全部が同時に上限まで使うことはない前提です。

- Valkey は**追い出しません**（`noeviction`）。取り消したセッションも入っているので、消えると取り消しが効かなくなります。
  `maxmemory` に達すると書き込みが断られ、サーバーのログに出ます。ふつうは 10 MB 程度です。
- PostgreSQL の接続は api・realtime・worker が 20 ずつ（`TOKULY_DATABASE_MAX_CONNECTIONS`）、残りは
  `docker compose run` とバックアップの分です。
- `observability` プロファイル（Grafana など、上限 1 GB）は 4 GB の予算に入っていません。
- 落ちて再起動したサービスは `docker inspect --format '{{.State.OOMKilled}} {{.RestartCount}}' tokuly-<サービス>-1` で分かります。
  何度も落ちるなら、そのサービスの上限を `.env` で上げます（変数名は [`compose.yaml`](../compose.yaml)）。
