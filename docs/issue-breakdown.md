# StorageSlim Issue 分解案

最終更新: 2026-08-22
ステータス: Draft

## 1. 目的

このファイルは、残っている検討事項や検証事項を GitHub Issue として起票しやすい粒度に分解したものです。

用途:

- Issue タイトルのたたき台にする
- 1 Issue あたりの完了条件を明確にする
- 依存関係を整理する

## 2. 起票方針

- 1 Issue は 1 論点に寄せる
- 実装 Issue と検証 Issue を分ける
- `何を決めれば閉じられるか` を明確にする
- 依存先があるものは先に検証 Issue を置く

推奨ラベル例:

- `type:decision`
- `type:research`
- `type:spec`
- `type:tech`
- `area:formats`
- `area:gif`
- `area:heic`
- `area:desktop`
- `area:libraries`

## 3. 優先度順の Issue 分解

### ISS-01 GIF の非アニメ形式変換方針を確定する

- 種別: decision
- 優先度: 高
- 推奨ラベル:
  - `type:decision`
  - `area:gif`
  - `area:formats`

目的:

- アニメーション GIF を JPEG / PNG / WebP / AVIF へ変換する場合の扱いを確定する

論点:

- 非アニメ形式への変換を禁止するか
- 先頭フレーム書き出しを許可するか
- アニメーション保持可能な形式のみ許可するか

完了条件:

- 初期版の正式方針が 1 つに決まっている
- UI 上の扱いとエラー文言方針が決まっている
- `docs/requirements.md` と `docs/decision-log.md` に反映されている

依存:

- なし

---

### ISS-02 出力形式の対応マトリクスを定義する

- 種別: spec
- 優先度: 高
- 推奨ラベル:
  - `type:spec`
  - `area:formats`

目的:

- 入力形式ごとに、どの出力形式を許可するかを表として確定する

論点:

- 全形式相互変換のうち、初期版で非対応にする組み合わせ
- GIF / HEIC / HEIF を含む特殊ケースの扱い
- `オリジナル形式を維持` の適用範囲

完了条件:

- 入力形式 x 出力形式の対応表が完成している
- 非対応組み合わせごとに理由が説明できる
- UI 制御の前提として使える

依存:

- `ISS-01`
- `ISS-04`

---

### ISS-03 Tauri 採用可否を確認する

- 種別: research
- 優先度: 高
- 推奨ラベル:
  - `type:research`
  - `type:tech`
  - `area:desktop`

目的:

- `StorageSlim` の要件に対して Tauri を第一候補として維持できるか確認する

論点:

- ファイル I/O
- 大量ファイル処理
- ネイティブ画像処理ライブラリ連携
- Windows / macOS の配布現実性

完了条件:

- Tauri 採用の可否が `採用 / 条件付き採用 / 不採用` のいずれかで判断されている
- 次点候補へ切り替える条件が定義されている
- `docs/decision-log.md` に結果が反映されている

依存:

- なし

---

### ISS-04 libvips 系ライブラリで必要形式を満たせるか確認する

- 種別: research
- 優先度: 高
- 推奨ラベル:
  - `type:research`
  - `type:tech`
  - `area:libraries`
  - `area:formats`

目的:

- 第一候補である `libvips 系` で、必要な入出力形式と圧縮要件を満たせるか確認する

論点:

- GIF 読込 / 書出し
- アニメーション GIF の保持
- HEIC / HEIF 読込 / 再出力
- AVIF / WebP / PNG / JPEG の基本対応
- メタデータ保持と削除の実現性

完了条件:

- 形式ごとの対応可否一覧が作成されている
- `単独で足りるか` または `補助ライブラリが必要か` が判断されている
- `docs/decision-log.md` に結果が反映されている

依存:

- なし

---

### ISS-05 HEIC / HEIF の Windows / macOS 実装制約を整理する

- 種別: research
- 優先度: 高
- 推奨ラベル:
  - `type:research`
  - `area:heic`
  - `area:formats`

目的:

- HEIC / HEIF の読込と `オリジナル形式維持` 再出力について、OS ごとの制約を整理する

論点:

- Windows と macOS での対応差分
- 再出力時の互換性期待値
- ライセンスや配布制約の有無

完了条件:

- Windows / macOS それぞれの可否と制約が表で整理されている
- 初期版で明記すべき制限が定義されている
- `docs/requirements.md` と `docs/decision-log.md` に反映できる状態になっている

依存:

- `ISS-04`

---

### ISS-06 フレームワークとライブラリの組み合わせを最終決定する

- 種別: decision
- 優先度: 高
- 推奨ラベル:
  - `type:decision`
  - `type:tech`
  - `area:desktop`
  - `area:libraries`

目的:

- デスクトップフレームワークと画像処理ライブラリの組み合わせを正式決定する

論点:

- `Tauri + libvips 系` を採用するか
- 補助ライブラリの追加前提を許容するか
- 必要なら Electron へ切り替えるか

完了条件:

- 採用構成が 1 つに決まっている
- 切り替え条件または非採用理由が明文化されている
- `docs/decision-log.md` に反映されている

依存:

- `ISS-03`
- `ISS-04`
- `ISS-05`

---

### ISS-07 出力形式マトリクスを requirements に反映する

- 種別: spec
- 優先度: 中
- 推奨ラベル:
  - `type:spec`
  - `area:formats`

目的:

- 検証と決定結果を `requirements.md` に反映し、仕様として固定する

論点:

- 対応表の記載方法
- 非対応理由の書き方
- `オリジナル形式を維持` の注記方法

完了条件:

- `docs/requirements.md` に最終対応マトリクスが反映されている
- `docs/open-questions.md` から該当項目が削除されている

依存:

- `ISS-01`
- `ISS-02`
- `ISS-05`
- `ISS-06`

## 4. 起票順の推奨

1. `ISS-01` GIF の非アニメ形式変換方針
2. `ISS-03` Tauri 採用可否確認
3. `ISS-04` libvips 系ライブラリ検証
4. `ISS-05` HEIC / HEIF 制約整理
5. `ISS-02` 出力形式マトリクス定義
6. `ISS-06` 技術構成の最終決定
7. `ISS-07` requirements 反映

## 5. 1 Issue の推奨テンプレート

```md
## 概要

## 背景

## 決めること / 調べること

## 完了条件

## 依存関係

## 関連ドキュメント
- docs/requirements.md
- docs/open-questions.md
- docs/decision-log.md
```

## 6. 動画圧縮モード対応の Issue 分解

`docs/decision-log.md` の `D-17` から `D-20`、および `docs/requirements-video.md` を前提とした分解です。

作業ブランチの方針:

- `ISS-08` / `ISS-09` は動画対応を断念しても価値が残るため `master` で進める。
- `ISS-10` 以降は `feature/video-mode` で進め、断念時はブランチごと破棄できる状態を保つ。

追加の推奨ラベル例:

- `area:video`
- `area:ffmpeg`
- `area:license`
- `area:refactor`
- `type:build`

### ISS-08 App.tsx から共通部品を抽出する

- 種別: tech
- 優先度: 高
- ブランチ: `master`
- 推奨ラベル:
  - `type:tech`
  - `area:refactor`

目的:

- 約 1770 行の `src/App.tsx` から、モードに依存しない部品を切り出す

対象:

- 入力一覧テーブル
- 結果テーブル
- 進捗パネル
- パス選択行
- `NumberField`
- ドロップゾーン
- 共通シェル（ヘッダー + `設定` / `入力と結果` の枠 + ストレッチとスクロール制御）

完了条件:

- 抽出後も既存の画像処理の振る舞いが変わっていない
- モードの概念を一切導入していない
- 差分が「移設のみ」であることをレビューで確認できる

依存:

- なし

---

### ISS-09 画像設定 UI を ImageSettingsPanel へ切り出す

- 種別: tech
- 優先度: 高
- ブランチ: `master`
- 推奨ラベル:
  - `type:tech`
  - `area:refactor`

目的:

- 画像固有の設定 UI とハンドラを 1 コンポーネントへまとめ、モード追加時の差分を局所化する

完了条件:

- 画像設定の描画と状態更新が `ImageSettingsPanel` に閉じている
- 振る舞いは変更していない
- 単独のコミットとして分離されている

依存:

- `ISS-08`

---

### ISS-10 モード切り替えの基盤を実装する

- 種別: tech
- 優先度: 高
- ブランチ: `feature/video-mode`
- 推奨ラベル:
  - `type:tech`
  - `area:video`

目的:

- `画像圧縮モード` / `動画圧縮モード` の切り替えを成立させる

対象:

- 最上位の `AppMode` 状態と `storageslim.mode.v1` への永続化
- 設定永続化キーの分離（`storageslim.settings.v1` / `storageslim.video.settings.v1`）
- 処理中のモード切り替え無効化
- モード別の入力一覧・結果の保持
- 対象外種別を理由付きで処理対象外へ落とす分岐（件数の集約表示を含む）

完了条件:

- モードを切り替えても、もう一方の入力一覧と結果が失われない
- 画像モードに動画を投入した場合、および逆の場合に理由が表示される
- 処理中はモード切り替えができない
- アプリ再起動後に前回のモードと各モードの設定が復元される

依存:

- `ISS-09`

---

### ISS-11 LGPL 構成の FFmpeg 取得とバンドルを整備する

- 種別: tech
- 優先度: 高
- ブランチ: `feature/video-mode`
- 推奨ラベル:
  - `type:build`
  - `area:ffmpeg`
  - `area:license`

目的:

- LGPL 構成の FFmpeg / ffprobe を sidecar として同梱できる状態にする

対象:

- 取得スクリプト（`src-tauri/binaries/` へ配置、ターゲットトリプル付きの命名）
- `.gitignore` への追加（バイナリは git 管理対象外）
- `tauri.conf.json` の `externalBin` 設定
- Windows は LGPL 版ビルドの利用、macOS は LGPL 構成での自前ビルド手順
- 起動時のバージョン確認と、利用不可時の表示
- `外部 ffmpeg のパス` 指定設定

完了条件:

- 同梱バイナリが GPL 成分（`libx264` / `libx265`）を含まないことを `ffmpeg -version` の configure 出力で確認できる
- Windows / macOS の両方でビルド済みアプリから FFmpeg を呼び出せる
- FFmpeg が無い状態でもアプリが起動し、動画モードで理由が表示される
- 外部パス指定が同梱バイナリより優先される

依存:

- なし（`ISS-10` と並行可）

---

### ISS-12 ffprobe による動画入力の判定と入力一覧を実装する

- 種別: tech
- 優先度: 高
- ブランチ: `feature/video-mode`
- 推奨ラベル:
  - `type:tech`
  - `area:video`

目的:

- 動画モードの入力読込と、入力一覧への情報表示・警告表示を実装する

対象:

- `inspect_video_inputs` コマンドと `VideoInputEntry`
- 拡張子に依存しない映像ストリーム判定
- 解像度・再生時間・fps・コデック・音声有無・ビットレート・回転・トラック数・HDR の取得
- 入力時警告（長尺 / 4K 以上 / HDR / 可変フレームレート / 副音声・字幕あり / 非対応コデック）

完了条件:

- `docs/requirements-video.md` の 4 章の情報が入力一覧で確認できる
- 拡張子が偽っているファイルを正しく判別できる
- 警告条件がすべて表示される

依存:

- `ISS-11`

---

### ISS-13 動画エンコード処理を実装する

- 種別: tech
- 優先度: 高
- ブランチ: `feature/video-mode`
- 推奨ラベル:
  - `type:tech`
  - `area:video`

目的:

- MP4 (H.264 + AAC) 出力を実装する

対象:

- `process_video_batch` コマンド
- 一時ファイル（`*.part`）への書き出しと成功時のリネーム
- リサイズ（偶数丸め・拡大なし）
- 品質プリセットからエンコーダ経路別の指定値へのマップ
- フレームレート上限と可変フレームレートの固定化
- 音声 3 択（コピー / AAC 再エンコード / 削除）
- メタデータ削除時の回転情報維持
- 先頭の映像 1 本 + 音声 1 本のみ出力（破棄したトラックは警告）
- サイズ増加時の元ファイルコピー
- タイムスタンプ引き継ぎ（既存実装の流用）

完了条件:

- 縦向き動画が縦向きで出力される
- 失敗時に不完全な出力ファイルが残らない
- CRF 対応経路と非対応経路の両方で出力できる
- 出力が元より大きい場合の扱いが画像モードと一致している

依存:

- `ISS-12`

---

### ISS-14 動画の進捗表示と停止・一時停止を実装する

- 種別: tech
- 優先度: 高
- ブランチ: `feature/video-mode`
- 推奨ラベル:
  - `type:tech`
  - `area:video`

目的:

- 長時間処理に耐える進捗と中断操作を実装する

対象:

- `-progress pipe:1` の `out_time_us / duration` によるファイル内進捗
- `BatchProgress` へのファイル内進捗率の追加と 2 段表示
- 停止時の stdin への `q` 送信、一時ファイル削除、`中断（出力を破棄しました）` の記録
- 停止・一時停止のボタン文言のモード別切り替え
- 再生時間に対して不自然に長い処理のタイムアウト監視

完了条件:

- 1 ファイル処理中に進捗率が更新される
- 停止操作から数秒で停止し、一時ファイルが残らない
- 結果一覧で中断と失敗を区別できる
- `q` 送信で終了しないケースのフォールバックが用意されている

依存:

- `ISS-13`

---

### ISS-15 動画モードの設定 UI を実装する

- 種別: tech
- 優先度: 中
- ブランチ: `feature/video-mode`
- 推奨ラベル:
  - `type:tech`
  - `area:video`

目的:

- `VideoSettingsPanel` を実装する

対象:

- 出力形式 / 出力先 / 上書き
- リサイズ
- 品質プリセット 4 段と結果サイズの目安表示
- フレームレート上限
- 音声
- メタデータ
- タイムスタンプ
- 詳細（CRF 直接指定、外部 ffmpeg パス）

完了条件:

- 画像モードの設定パネルと並び順が揃っている
- CRF 非対応のエンコーダ選択時に CRF 入力が無効化され、理由が表示される
- 画像側の `デコード上限 (MB)` が動画モードに出ていない

依存:

- `ISS-10`
- `ISS-13`

---

### ISS-16 エンコーダ経路別の品質とサイズを比較検証する

- 種別: research
- 優先度: 高
- ブランチ: `feature/video-mode`
- 推奨ラベル:
  - `type:research`
  - `area:video`

目的:

- OS / ハードウェアエンコーダの H.264 出力が実用水準にあるかを判断する

論点:

- 同一素材で、各経路（Media Foundation / NVENC / QSV / AMF / VideoToolbox）の品質とサイズがどう違うか
- 品質プリセットのマップ値が妥当か
- 結果サイズの目安表示の誤差がどの程度か

完了条件:

- 代表素材での比較結果が記録されている
- プリセットのマップ値が確定している
- 実用水準に達しない場合の対応方針（`D-18` の保留条件）が判断されている

依存:

- `ISS-13`

---

### ISS-17 ライセンス表記と配布物を整備する

- 種別: spec
- 優先度: 高
- ブランチ: `feature/video-mode`
- 推奨ラベル:
  - `type:spec`
  - `area:license`

目的:

- MIT 公開と有償ビルド配布の両方で、LGPL の義務を満たす状態にする

対象:

- `THIRD-PARTY-NOTICES.md` への LGPL 2.1 全文・FFmpeg バージョン・configure オプション・同一リビジョンのソース入手先の記載
- `scripts/generate-notices.mjs` が走査しない FFmpeg 分の管理方法
- `README.md` への動画モードとライセンス構成の追記
- 配布時の商品説明に追加制限（再配布禁止等）を記載しない方針の明文化
- macOS の同梱バイナリ署名と公証の確認

完了条件:

- 同梱物のライセンス条件が配布物内で確認できる
- ソース入手先が実際に辿れる
- macOS のビルドが署名済みで起動する

依存:

- `ISS-11`

---

### 起票順の推奨（動画対応）

1. `ISS-08` 共通部品の抽出（master）
2. `ISS-09` 画像設定 UI の切り出し（master）
3. `ISS-11` FFmpeg のバンドル整備
4. `ISS-10` モード切り替えの基盤
5. `ISS-12` 動画入力の判定
6. `ISS-13` エンコード処理
7. `ISS-14` 進捗と停止
8. `ISS-15` 設定 UI
9. `ISS-16` エンコーダ経路の検証
10. `ISS-17` ライセンス表記と配布物
