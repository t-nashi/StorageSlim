# StorageSlim Issue 分解案

最終更新: 2026-08-10
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
