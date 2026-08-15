# AVIF 寸法上限 境界テスト画像

rav1e の寸法制限 **1 〜 65,535 px**（幅・高さそれぞれ独立）を境界としたテスト用画像です。

## 制限の出どころ

`rav1e-0.8.1/src/api/config/mod.rs:310-322`

```rust
if (config.still_picture && config.width < 1)
  || (!config.still_picture && config.width < 16)
  || config.width > u16::MAX as usize
{
  return Err(InvalidWidth(config.width));
}
```

動画用途（`still_picture = false`）では下限が 16 px ですが、
ravif は `still_picture: true` を設定するため（`ravif-0.13.0/src/av1encoder.rs:671`）、
**AVIF 出力では下限は 1 px** です。1 px の画像でも通ります。
実質的な境界は上限側の **65,535 / 65,536** のみです。

## ファイル一覧

| ファイル | 寸法 | 期待結果 (出力形式: AVIF) |
|---|---|---|
| `wide-65535x64.png` | 65535 x 64 | **成功** |
| `wide-65536x64.png` | 65536 x 64 | **失敗**（幅が 1 px 超過） |
| `tall-64x65535.png` | 64 x 65535 | **成功** |
| `tall-64x65536.png` | 64 x 65536 | **失敗**（高さが 1 px 超過） |

いずれも 4.2 メガピクセル程度（デコード時 約 12.6 MB / RGBA 化で 約 16.8 MB）なので、
`image` クレートのデコード上限 512 MiB には掛かりません。寸法制限のみを単独で検証できます。

## 手順

1. 出力形式を **AVIF** に設定
2. リサイズ基準を **変更なし** に設定
3. 4 ファイルを入力一覧へ追加して実行

成功する 2 件は 4.2 メガピクセルを speed 6 で AVIF エンコードするため、
WebP のテストより時間が掛かります。

## WebP テストとの違い

### 1. パニックではなくエラーになる

`webp` クレートと違い、`image` の `AvifEncoder` は `Result` を返します
(`image-0.25.10/src/codecs/avif/encoder.rs:127`)。結果欄には次のように出るはずです。

```
Format error encoding Avif:
Encoding error reported by rav1e
```

`processing panicked:` では**ない**点が WebP との差です。

なお ravif の `Error::EncodingError` は `rav1e::InvalidConfig` の中身を捨てるため
(`ravif-0.13.0/src/error.rs:19-23`)、`invalid width 65536` という具体的な理由までは出ません。

### 2. 失敗しても 0 byte ファイルが残る

AVIF 分岐 (`src-tauri/src/lib.rs:597`) は `File::create()` を**エンコード前**に実行します。

```rust
OutputFormat::Avif => {
    let mut writer = BufWriter::new(File::create(output_path)?);  // <- 先に作られる
    ...
    encoder.write_image(...)?;                                    // <- ここで失敗
}
```

そのため失敗した 2 件についても、出力先に **0 byte の `.avif` ファイルが残ります**。
WebP 分岐は `fs::write` を後に行うためこの問題がありません
（JPEG / PNG 分岐は AVIF と同じ順序なので同様に残ります）。

テスト時は出力先フォルダを確認してください。

## 対照テスト

`wide-65536x64.png` に対して **リサイズ基準 = 幅 / 65000 px** を指定すると成功します。
寸法起因であることの確認に使えます。

## 実用上の上限は 32,768 px（デコーダ側の制限・実測確認済み）

エンコード可能な寸法と、実際に開ける寸法は一致しません。

| | 上限 | 根拠 |
|---|---|---|
| rav1e（エンコード側・本アプリ） | **65,535 px** | `rav1e-0.8.1/src/api/config/mod.rs:310` |
| libavif（デコード側の既定値） | **32,768 px** | `AVIF_DEFAULT_IMAGE_DIMENSION_LIMIT` |
| libheif（ImageMagick 等） | より緩く読める | 実測で 65,535 px を読めた |

libavif は Chrome / Firefox / Windows / IrfanView など主要ビューアが採用しているため、
**32,769 〜 65,535 px の AVIF はアプリ上「成功」と表示されても、ほとんどのソフトで開けません。**

libavif が上限超過時に返す `AVIF_RESULT_BMFF_PARSE_FAILED` は
`Can't decode image: BMFF parsing failed` として表示されます。

### 実測結果（IrfanView / Windows）

`viewer-check/` の 2 ファイルで境界を確認済みです。

| ファイル | 寸法 | IrfanView |
|---|---|---|
| `viewer-check/viewer-64x32768.avif` | 64 x 32768 | **開ける** |
| `viewer-check/viewer-64x32769.avif` | 64 x 32769 | **開けない** (BMFF parsing failed) |

一方、本アプリが出力した `64 x 65535` の AVIF は ImageMagick (libheif 1.23.1) では
正常に読めており、`ftyp avif` / `ispe width=64 height=65535` (u32) / `av01` とも
構造は正常です。**出力ファイル自体は壊れていません。**

なお `viewer-check/` の 2 ファイルは ImageMagick で直接生成したもので、
本アプリの出力ではありません（ビューア側の境界確認専用）。

## 生成コマンド

```bash
magick -size 64x65535 gradient:'#ffd60a-#ff375f' -rotate 90 -depth 8 -define png:color-type=2 wide-65535x64.png
magick -size 64x65536 gradient:'#ffd60a-#ff375f' -rotate 90 -depth 8 -define png:color-type=2 wide-65536x64.png
magick -size 64x65535 gradient:'#30d158-#64d2ff' -depth 8 -define png:color-type=2 tall-64x65535.png
magick -size 64x65536 gradient:'#30d158-#64d2ff' -depth 8 -define png:color-type=2 tall-64x65536.png
```
