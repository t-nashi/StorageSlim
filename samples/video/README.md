# StorageSlim 動画サンプル

動画圧縮モードの動作確認に使うクリップの作り方をまとめています。
生成物はサイズが大きいためリポジトリに含めず、必要なときに作ります。

実行手順は `docs/verification-guide.md` の動画モードの節を参照してください。

## 生成コマンド

`ffmpeg` が PATH にある状態で、このディレクトリで実行します。

```bash
# 基本確認: 1280x720 30fps 6 秒 + 音声つき
ffmpeg -y -f lavfi -i testsrc=size=1280x720:rate=30:duration=6 \
  -f lavfi -i sine=frequency=440:duration=6 \
  -c:v libx264 -crf 18 -c:a aac -pix_fmt yuv420p basic.mp4

# 進捗と停止の確認: 4K 60fps 3 分（1 件が長いものが必要）
ffmpeg -y -f lavfi -i testsrc2=size=3840x2160:rate=60:duration=180 \
  -c:v libx264 -crf 20 -pix_fmt yuv420p -an long-4k.mp4

# 音声コピー不可の確認: MP4 へそのまま入れられない音声
ffmpeg -y -f lavfi -i testsrc=size=640x360:rate=30:duration=5 \
  -f lavfi -i sine=frequency=440:duration=5 \
  -c:v libx264 -crf 24 -c:a libopus -pix_fmt yuv420p opus-audio.webm

# サイズ増加の確認: 既に小さく圧縮済みのもの
ffmpeg -y -f lavfi -i testsrc=size=320x240:rate=15:duration=5 \
  -c:v libx264 -crf 40 -pix_fmt yuv420p -an tiny.mp4

# 音声なし
ffmpeg -y -f lavfi -i testsrc=size=854x480:rate=25:duration=5 \
  -c:v libx264 -crf 24 -pix_fmt yuv420p -an silent.mp4
```

## 合成では作れないもの

次の 2 つは実機で撮った動画が必要です。

- **縦向き動画（回転メタデータつき）**: スマートフォンで撮影した縦動画。出力が横倒しにならないかを確認する。`ffmpeg` の `lavfi` では回転メタデータを持つファイルを作れない。
- **HDR (HLG / PQ)**: 対応端末で撮影したもの。警告表示と、色が破綻しないかを確認する。

## 対象外種別の確認

動画モードに画像を入れたときの挙動は、`samples/input/` の画像をそのまま使えます。
`対象外の種別 N 件` として集約表示されることを確認します。
