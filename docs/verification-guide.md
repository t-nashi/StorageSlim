# StorageSlim 動作検証手順書

最終更新: 2026-08-15

`samples/` の画像を使った動作確認・回帰確認の手順です。
起動や基本 UI の確認は `docs/debug-sheet.md`、形式ごとの仕様は `docs/format-matrix.md` を参照してください。

## 0. 準備

### ビルド

```powershell
npm run tauri build
```

必ず Tauri CLI 経由でビルドします。`cargo build` で作った exe は debug / release を問わずフロントエンドが同梱されず、`http://localhost:1420` を見にいくため「ローカルに接続できません」で起動しません。

debug 版を単体で動かす場合:

```powershell
npm run tauri build -- --debug
```

開発中に Vite サーバーと同時起動する場合:

```powershell
npm run tauri dev
```

### 自動テスト

```powershell
cd src-tauri
cargo test --lib -- --skip inspect_desktop_samples_reports_expected_flags
```

```powershell
npx tsc --noEmit
```

`inspect_desktop_samples_reports_expected_flags` はローカルの `Desktop\@StorageSlim\input` に特定のフィクスチャがある前提のテストです。環境依存のため通常はスキップします。

### 共通の注意

- AVIF は処理が重いため release ビルドで確認します。
- `上書きを許可する` が OFF のまま繰り返すと出力に `_1` `_2` の連番が付きます。判断に迷う場合は出力先を空にしてから実行してください。
- 設定変更後、**既に読み込み済みの入力一覧は再評価されません**。デコード上限を変えた場合は `入力をクリア` してから読み込み直してください。

---

## 1. 寸法上限（WebP）

**目的**: libwebp の 16,383 px 制限を、パニックではなく理由の分かるエラーとして扱えているか。

1. 出力形式 **WebP**、リサイズ基準 **変更なし**
2. `samples/webp-limit/` の 4 ファイルを追加
3. 実行

| ファイル | 期待結果 |
| --- | --- |
| `wide-16383x64.png` | success |
| `tall-64x16383.png` | success |
| `wide-16384x64.png` | failed `WebP は幅・高さとも 16383 px までです（出力予定 16384 x 64）。…` |
| `tall-64x16384.png` | failed（同様） |

**確認ポイント**

- 失敗理由が `processing panicked: called 'Result::unwrap()' on an 'Err' value: VP8_ENC_ERROR_BAD_DIMENSION` に**なっていない**こと
- 出力先に 0 byte の `.webp` が**残っていない**こと

**対照テスト**: `wide-16384x64.png` にリサイズ基準 `幅 / 16000 px` を指定すると成功します。寸法起因であることの確認に使えます。

---

## 2. 寸法上限（JPEG）

**目的**: 規格上限 65,535 ではなく libjpeg の 65,500 で切れているか。

1. 出力形式 **JPEG**、リサイズ基準 **変更なし**
2. `samples/jpeg-limit/` の 2 ファイルを追加
3. 実行

| ファイル | 期待結果 |
| --- | --- |
| `wide-65500x64.png` | success |
| `wide-65501x64.png` | failed `JPEG は幅・高さとも 65500 px までです…` |

**確認ポイント**

成功した `wide-65500x64.jpg` が**ビューアで開けること**。65,501 以上で書けてしまうと、`Maximum supported image dimension is 65500 pixels` で開けないファイルが生成されます。

---

## 3. 寸法上限と警告しきい値（AVIF）

**目的**: 失敗（65,535 超）と警告（32,768 超）が正しく切り替わるか。

1. 出力形式 **AVIF**、リサイズ基準 **変更なし**
2. `samples/avif-limit/` の 6 ファイルを追加
3. 実行

| ファイル | 期待結果 | 警告 |
| --- | --- | --- |
| `threshold-64x32768.png` | success | **なし** |
| `threshold-64x32769.png` | success | **あり** |
| `tall-64x65535.png` | success | あり |
| `wide-65535x64.png` | success | あり |
| `tall-64x65536.png` | failed | - |
| `wide-65536x64.png` | failed | - |

警告タグの文言: `主要ビューア上限 32768 px 超`

**確認ポイント**

- 32768 と 32769 の**境界で切り替わる**こと。ここがこのテストの主目的です
- 失敗した 2 件について、出力先に 0 byte の `.avif` が**残っていない**こと
- 警告タグで 状態 列が横に伸びすぎていないこと

**補足**: 警告付きで成功した AVIF は IrfanView などで開けません。これは想定どおりで、ファイル自体は仕様上正常です（ImageMagick / libheif では読めます）。`samples/avif-limit/viewer-check/` の 2 ファイルでビューア側の境界（32768 は開ける / 32769 は開けない）を確認できます。

---

## 4. アニメーション GIF のリサイズ

**目的**: 部分矩形フレームを持つ GIF が正しくリサイズされるか。

1. 出力形式 **GIF**、リサイズ基準 **長辺 / 50 %**
2. `samples/animated/offset-frames.gif` を追加（200 x 120、2 コマ目以降が `96x66+10+20` などの部分矩形）
3. 実行

**確認ポイント**

- 出力が **100 x 60** になっていること
- アニメーションを再生し、**オレンジの四角が左から右へ移動**すること
- 四角が左上に固定されていない、極端に小さくなっていない、キャンバスに余白が残っていないこと

修正前は、キャンバスが 200x120 のまま・全フレームが左上配置・部分矩形フレームが過剰に縮小、という 3 つの破綻が同時に起きていました。

`samples/input/sample-animated.gif` は全フレームが全面のため、この不具合は検出できません。

---

## 5. サイズ増加の表示

**目的**: 出力が元より大きくなった場合に増加が明示されるか。

1. 出力形式 **JPEG**、リサイズ基準 **変更なし**
2. `samples/size-increase/` の 2 ファイルを追加
3. 実行

| ファイル | 期待表示 |
| --- | --- |
| `flat-320x200.png` | SAVED 列が `+◯◯ B / +◯◯%`（オレンジ） |
| `gradient-256x256.png` | 同上 |

**確認ポイント**

- `0 B / 0.0%` に**なっていない**こと
- 合計欄が `増加: ◯◯ / +◯◯%` になること
- 削減されるケースと配色で区別できること

平坦な画像を JPEG に変換すると増えるのは正常な結果です。問題は「増えたことが表示されないこと」でした。

---

## 6. デコード上限

**目的**: 事前警告・エラーメッセージ・上限調整が機能するか。

### 6-1. 事前警告

1. デコード上限を既定の **512** にする
2. `samples/decode-limit/huge-24000x8000.png` を追加

入力一覧の 状態 列に `デコードに約 732 MB 必要 (上限 512 MB)` が表示されること。**実行前**に出ることが要件です。

### 6-2. エラーメッセージ

そのまま実行し、失敗理由が次のようになること。

```
failed to decode image: Memory limit exceeded
```

`failed to decode image` だけで終わっていないこと（エラーチェーンが表示されること）を確認します。

### 6-3. 上限調整

1. `品質調整・その他` を開く
2. `デコード上限` を **2048** に変更
3. `入力をクリア` してから同じファイルを再度追加

- 入力一覧の警告が**消える**こと
- 実行して成功すること

### 6-4. 入力値のクランプ

| 入力 | 期待 |
| --- | --- |
| `10` | フォーカスを外すと `64` |
| `99999` | フォーカスを外すと `8192` |
| 空欄 | フォーカスを外すと `512` |

### 6-5. UI 表示

- `既定 512 / 範囲 64-8192` が常時表示されること
- ウィンドウ幅 1500px 以上で補足文（`大きな画像の読込に必要な量。上げすぎると…`）が表示されること
- ウィンドウを狭めると補足文だけが消え、既定値・範囲は残ること

---

## 7. 回帰確認（既存機能）

新しい確認項目を追加した際に壊れていないか見る最低限のセットです。

| 項目 | 手順 | 期待 |
| --- | --- | --- |
| 基本変換 | `samples/input/` を JPEG 出力 | 成功、削減率が表示される |
| アニメーション制約 | `sample-animated.gif` を PNG 出力 | 失敗「アニメーション GIF は GIF 以外の形式へ変換できません。」 |
| HEIC 読込 | `sample-heic.heic` を JPEG 出力 | 成功 |
| HEIC オリジナル維持 | `sample-heic.heic` をオリジナル維持 | 成功、コピー扱いの警告 |
| AVIF 入力制限 | `sample-avif.avif` を追加 | `制約` 表示、処理対象外 |
| リサイズ必須 | 基準を「幅」にして値を空にする | 実行ボタンが無効 |
| 連番付与 | 上書き OFF で 2 回実行 | 2 回目に `_1` が付く |
| 一時停止 / 停止 | 処理中に操作 | 処理中ファイル完了後に反映 |

---

## 付録: サンプル画像の生成コマンド

すべて ImageMagick 7 で生成しています。

```bash
# WebP 境界 (16383 / 16384)
magick -size 64x16383 gradient:'#ff3b30-#0a84ff' -rotate 90 -depth 8 -define png:color-type=2 samples/webp-limit/wide-16383x64.png
magick -size 64x16384 gradient:'#ff3b30-#0a84ff' -rotate 90 -depth 8 -define png:color-type=2 samples/webp-limit/wide-16384x64.png
magick -size 64x16383 gradient:'#34c759-#af52de' -depth 8 -define png:color-type=2 samples/webp-limit/tall-64x16383.png
magick -size 64x16384 gradient:'#34c759-#af52de' -depth 8 -define png:color-type=2 samples/webp-limit/tall-64x16384.png

# JPEG 境界 (65500 / 65501)
magick -size 64x65500 gradient:'#af52de-#ffd60a' -rotate 90 -depth 8 -define png:color-type=2 samples/jpeg-limit/wide-65500x64.png
magick -size 64x65501 gradient:'#af52de-#ffd60a' -rotate 90 -depth 8 -define png:color-type=2 samples/jpeg-limit/wide-65501x64.png

# AVIF 境界 (65535 / 65536) と警告しきい値 (32768 / 32769)
magick -size 64x65535 gradient:'#ffd60a-#ff375f' -rotate 90 -depth 8 -define png:color-type=2 samples/avif-limit/wide-65535x64.png
magick -size 64x65536 gradient:'#ffd60a-#ff375f' -rotate 90 -depth 8 -define png:color-type=2 samples/avif-limit/wide-65536x64.png
magick -size 64x65535 gradient:'#30d158-#64d2ff' -depth 8 -define png:color-type=2 samples/avif-limit/tall-64x65535.png
magick -size 64x65536 gradient:'#30d158-#64d2ff' -depth 8 -define png:color-type=2 samples/avif-limit/tall-64x65536.png
magick -size 64x32768 gradient:'#5ac8fa-#ff9f0a' -depth 8 -define png:color-type=2 samples/avif-limit/threshold-64x32768.png
magick -size 64x32769 gradient:'#5ac8fa-#ff9f0a' -depth 8 -define png:color-type=2 samples/avif-limit/threshold-64x32769.png

# ビューア側の上限確認 (アプリ出力ではない)
magick -size 64x32768 gradient:'#0a84ff-#ffd60a' -depth 8 samples/avif-limit/viewer-check/viewer-64x32768.avif
magick -size 64x32769 gradient:'#0a84ff-#ffd60a' -depth 8 samples/avif-limit/viewer-check/viewer-64x32769.avif

# 部分矩形フレームを持つアニメーション GIF
magick -delay 20 -loop 0 \
  \( -size 200x120 xc:'#102030' -fill '#ff9f0a' -draw "rectangle 10,20 60,70" \) \
  \( -size 200x120 xc:'#102030' -fill '#ff9f0a' -draw "rectangle 55,35 105,85" \) \
  \( -size 200x120 xc:'#102030' -fill '#ff9f0a' -draw "rectangle 100,20 150,70" \) \
  \( -size 200x120 xc:'#102030' -fill '#ff9f0a' -draw "rectangle 145,35 195,85" \) \
  -layers OptimizeFrame samples/animated/offset-frames.gif

# サイズ増加の確認用
magick -size 320x200 xc:'#101820' -fill '#ffd60a' -draw "rectangle 40,40 280,160" -depth 8 -define png:color-type=2 samples/size-increase/flat-320x200.png
magick -size 256x256 gradient:'#0a84ff-#30d158' -depth 8 -define png:color-type=2 samples/size-increase/gradient-256x256.png

# デコード上限の確認用 (192 メガピクセル)
magick -size 24000x8000 gradient:'#ff375f-#5ac8fa' -depth 8 -define png:color-type=2 samples/decode-limit/huge-24000x8000.png
```

生成後の確認:

```bash
magick identify samples/animated/offset-frames.gif
```

2 コマ目以降が `96x66 200x120+10+20` のように**オフセット付きの部分矩形**になっていることを確認します。全フレームが `+0+0` だと、アニメーション GIF の回帰確認になりません。
