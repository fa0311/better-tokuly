# 同梱しているサードパーティのソフトウェア

配布している `backend` イメージには、動画の変換に使う FFmpeg（[wader/static-ffmpeg](https://github.com/wader/static-ffmpeg)
のビルド、x264 / x265 入り）が別プログラムとして入っています。この FFmpeg は GPL-3.0-or-later です。
Tokuly 自身のコードは MIT のままですが、イメージを再配布するときは FFmpeg 部分に GPL の条件
（ライセンス文の同梱とソースコードの入手方法の案内。ソースは上記リポジトリと
[ffmpeg.org](https://ffmpeg.org/download.html) にあります）が付いてきます。
