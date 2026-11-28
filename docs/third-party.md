# 同梱しているサードパーティのソフトウェア

配布している `backend` イメージには、動画の変換に使う FFmpeg（[wader/static-ffmpeg](https://github.com/wader/static-ffmpeg)
のビルド、x264 / x265 入り）が別プログラムとして入っています。この FFmpeg は GPL-3.0-or-later です。
Tokuly 自身のコードは MIT のままですが、イメージを再配布するときは FFmpeg 部分に GPL の条件
（ライセンス文の同梱とソースコードの入手方法の案内。ソースは上記リポジトリと
[ffmpeg.org](https://ffmpeg.org/download.html) にあります）が付いてきます。

時間帯の境界（`tokuly-geo` が `tzf-dist` として組み込み）は
[timezone-boundary-builder](https://github.com/evansiroky/timezone-boundary-builder) のデータで、
© OpenStreetMap contributors、[ODbL 1.0](https://opendatacommons.org/licenses/odbl/1-0/) です。
