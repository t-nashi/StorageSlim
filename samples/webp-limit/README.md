# WebP 寸法上限 境界テスト画像

libwebp の `WEBP_MAX_DIMENSION` = **16,383 px** を境界としたテスト用画像です。
幅・高さそれぞれ独立に上限が適用されます。

## 重要: 正方形ではテストにならない

`16383 x 16383` / `16384 x 16384` のような正方形を使うと、**WebP エンコーダに到達する前に
デコード段階で失敗**します。

- `16383 x 16383` = 268,402,689 px x 3 byte (Rgb8) = **768 MiB**
- `image` クレートのデフォルト `max_alloc` = **512 MiB**

両方とも `failed to decode image` になり、WebP の寸法上限を検証できません。
そのため、本ディレクトリの画像は**メモリ消費が小さい細長いストリップ**にしています
(最大でも 16384 x 64 x 4 = 約 4.2 MB)。

## ファイル一覧

| ファイル | 寸法 | 期待結果 (出力形式: WebP) |
|---|---|---|
| `wide-16383x64.png` | 16383 x 64 | **成功** |
| `wide-16384x64.png` | 16384 x 64 | **失敗** (幅が 1 px 超過) |
| `tall-64x16383.png` | 64 x 16383 | **成功** |
| `tall-64x16384.png` | 64 x 16384 | **失敗** (高さが 1 px 超過) |

## 手順

1. 出力形式を **WebP** に設定
2. リサイズ基準を **変更なし** に設定 (リサイズすると上限を下回り再現しない)
3. 4 ファイルを入力一覧へ追加して実行

## 失敗時に想定されるエラー表示

`webp` クレートの `Encoder::encode()` は内部で `.unwrap()` しているため
(`webp-0.3.1/src/encoder.rs:58`)、エラーではなく**パニック**します。
`catch_task_panic` (`src-tauri/src/lib.rs:379`) が捕捉するのでアプリは落ちませんが、
結果欄には次のような文字列が出ます。

```
processing panicked: called `Result::unwrap()` on an `Err` value: VP8_ENC_ERROR_BAD_DIMENSION
```

WebP 分岐 (`src-tauri/src/lib.rs:590`) は `fs::write` の前にエンコードするため、
失敗しても 0 byte ファイルは残りません。
(JPEG / PNG / AVIF 分岐は `File::create` を先に行うため、この点の挙動が異なります)

## 対照テスト

`wide-16384x64.png` に対して **リサイズ基準 = 幅 / 16000 px** を指定すると、
上限を下回るため WebP 出力が成功します。寸法起因であることの確認に使えます。

## 生成コマンド

```bash
magick -size 64x16383 gradient:'#ff3b30-#0a84ff' -rotate 90 -depth 8 -define png:color-type=2 wide-16383x64.png
magick -size 64x16384 gradient:'#ff3b30-#0a84ff' -rotate 90 -depth 8 -define png:color-type=2 wide-16384x64.png
magick -size 64x16383 gradient:'#34c759-#af52de' -depth 8 -define png:color-type=2 tall-64x16383.png
magick -size 64x16384 gradient:'#34c759-#af52de' -depth 8 -define png:color-type=2 tall-64x16384.png
```
