# StorageSlim

StorageSlim は、画像ファイルをローカル PC 上で圧縮・リサイズ・形式変換するための Tauri 製デスクトップアプリです。Windows / macOS 両対応を前提に、クラウドへアップロードする前の写真・画像を手元で最適化することを目的としています。

## 現在の位置づけ

- ステータス: MVP 開発中
- フロントエンド: React + TypeScript + Vite
- デスクトップ: Tauri 2
- 画像処理: Rust backend
- 基本方針: ローカル処理、フォルダ構造維持、失敗ファイルを明示しながら成功分は保存

## 主な機能

- ファイル追加、フォルダ追加、ドラッグ&ドロップによる入力追加
- 入力先フォルダを指定して「読込」ボタンで対象画像を一覧化
- フォルダ投入時の再帰的探索
- 出力先フォルダ指定と既定値へのリセット
- 出力形式: オリジナル維持 / GIF / JPEG / PNG / WebP / AVIF
- リサイズ: 変更なし / 幅 / 高さ / 長辺
- リサイズ値: px / % 切り替え
- 品質調整:
  - JPEG: 品質 1-100
  - WebP: 品質 1-100
  - AVIF: 品質 1-100
  - PNG: 圧縮レベル 0-9
  - GIF: 色数 2-256
- メタデータ: 削除 / 保持
- タイムスタンプ: 作成日時 / 更新日時の引き継ぎ
- 上書き許可の切り替え
- 処理中の一時停止 / 再開 / 停止
- 結果一覧で Original / Optimized / Saved / 状態を表示

## 対応形式

| 形式 | 入力 | 出力 | 備考 |
| --- | --- | --- | --- |
| GIF | 可 | 可 | アニメーション GIF は GIF 出力のみ許可 |
| JPEG / JPG | 可 | 可 | 品質指定対応 |
| PNG | 可 | 可 | 圧縮レベル指定対応 |
| WebP | 可 | 可 | 品質指定対応 |
| AVIF | 認識可 / 処理は一時制限中 | 可 | AVIF 入力の処理は安定性確保のため現ビルドでは制限中 |
| HEIC | 可 | 明示的な HEIC 出力は不可 | オリジナル維持はコピー、他形式への変換は対応 |
| HEIF | 可 | 明示的な HEIF 出力は不可 | オリジナル維持はコピー、他形式への変換は対応 |

## 処理仕様

- 出力先は既定で `<ユーザーのデスクトップ>/@StorageSlim/output`。
- 入力先は既定で `<ユーザーのデスクトップ>/@StorageSlim/input`。
- 入力先 / 出力先や処理対象以外の設定はアプリ再起動後も保持する。
- 出力時は入力フォルダからの相対パスを維持し、出力フォルダ配下へ保存する。
- 上書き禁止時に同名ファイルがある場合は連番を付与する。
- 失敗したファイルがあっても、成功分は保存して処理を継続する。
- 停止操作は現在処理中のファイル完了後に反映する。エンコード中のファイルを途中破棄しないための仕様。

## 開発環境

必要なもの:

- Node.js / npm
- Rust / Cargo
- Tauri 2 が要求する OS 別のビルド環境

依存関係の取得:

```powershell
npm install
```

## Debug 実行

開発中にログやクラッシュ原因を確認しやすい実行方法です。Windows では debug exe 起動時にターミナルウィンドウも表示されることがあります。

Windows では PowerShell で実行することを前提にしています。macOS では Terminal.app、iTerm2 などのシェル環境で実行してください。

```powershell
npm run tauri build -- --debug
```

生成される主な実行ファイル:

```text
<repo>/src-tauri/target/debug/storageslim.exe
```

バンドルも生成されます:

```text
<repo>/src-tauri/target/debug/bundle/msi/StorageSlim_<version>_x64_en-US.msi
<repo>/src-tauri/target/debug/bundle/nsis/StorageSlim_<version>_x64-setup.exe
```

開発サーバー経由で起動する場合:

```powershell
npm run tauri dev
```

## Release ビルド

通常配布・実運用に近い確認用です。Windows では debug exe と異なり、通常はアプリ本体だけが起動し、開発用ターミナルウィンドウは表示されません。

ビルドコマンドは Windows では PowerShell、macOS では Terminal.app などのシェル環境で実行します。生成後のアプリ本体は、Windows では `storageslim.exe` またはインストーラー、macOS では生成された `.app` / `.dmg` などのバンドルから起動します。

```powershell
npm run tauri build
```

生成される主な実行ファイル:

```text
<repo>/src-tauri/target/release/storageslim.exe
```

生成されるインストーラー:

```text
<repo>/src-tauri/target/release/bundle/
```

AVIF 出力など重い処理の速度確認は、debug ではなく release で行うことを推奨します。

## 確認コマンド

フロントエンドの型チェックとビルド:

```powershell
npm run build
```

Rust 側のテスト:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
```

現在のローカル実データに依存するサンプルテストだけ除外する場合:

```powershell
cd src-tauri
cargo test --lib -- --skip inspect_desktop_samples_reports_expected_flags
```

## 注意事項

- AVIF 出力は JPEG / PNG / WebP より処理時間が長くなりやすい。
- AVIF の speed プリセットは現時点では UI に出さない。品質指定の意味が分かりにくくなるため、MVP では `AVIF 品質（1-100）` のみを提供する。
- HEIC / HEIF のオリジナル維持出力は再圧縮せずコピーする。リサイズやメタデータ削除とは両立しない場合がある。
- メタデータ保持は形式やライブラリ制約によりベストエフォート。
