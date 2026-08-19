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
- デコード上限: 64-8192 MB（既定 512 MB。大きな画像の読込に確保を許すメモリ量）
- メタデータ: 削除 / 保持
- タイムスタンプ: 作成日時 / 更新日時の引き継ぎ
- 上書き許可の切り替え
- 処理中の一時停止 / 再開 / 停止
- 結果一覧で Original / Optimized / Saved / 状態を表示
- 出力が元より大きくなった場合は `+◯◯ KB / +◯◯%` として増加を明示
- 現在の設定で失敗・注意が予測される入力は、実行前に入力一覧へ警告を表示（出力形式やリサイズ値を変えると即座に更新される）

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

## 寸法・メモリの制限

形式ごとに扱える寸法の上限が異なります。エンコーダが受け付ける値と、実際に開けるかどうかは一致しないため、両方を区別して扱っています。

| 出力形式 | 上限 | 挙動 | 根拠 |
| --- | --- | --- | --- |
| WebP | 16,383 px | 超えると失敗 | libwebp `WEBP_MAX_DIMENSION` |
| JPEG | 65,500 px | 超えると失敗 | libjpeg `JPEG_MAX_DIMENSION`。規格上は 65,535 だが libjpeg は 65,500 で切っている |
| GIF | 65,535 px | 超えると失敗 | ヘッダの寸法フィールドが 16bit |
| AVIF | 65,535 px | 超えると失敗 | rav1e が受け付ける上限 |
| AVIF | 32,768 px | **成功するが警告** | libavif 系デコーダの既定上限。これを超えた AVIF は主要ビューアで開けない |
| PNG | 制限なし | - | 寸法フィールドが 32bit |

幅・高さそれぞれに独立して適用されます。判定はリサイズ後の寸法に対して行うため、リサイズで上限以下に収めれば出力できます。

AVIF の 32,768 px だけ「失敗」ではなく「警告付きの成功」にしています。ファイル自体は仕様上正常で libheif 系のツールでは読めるため、失敗扱いにはしていません。ただし Chrome / Firefox / Windows / IrfanView などは libavif を使うため、実用上はほとんどのソフトで開けません。

### デコード時のメモリ上限

寸法が上限内でも、デコードに必要なメモリが確保できなければ処理できません。既定は 512 MB で、`品質調整・その他` から 64-8192 MB の範囲で変更できます。

必要量の目安は `幅 x 高さ x 3〜4 byte` です。上限を超える入力は、処理前に入力一覧へ `デコードに約 2.7 GB 必要 (上限 512 MB)` のように表示されます。

上限を上げる場合の注意:

- リサイズを併用すればピークはデコード分のみで済む。リサイズなしで PNG / WebP / AVIF へ出力すると、さらに RGBA 変換分が加算される。
- 実メモリを超える値を指定すると、メモリ確保の失敗はキャッチできずプロセスごと終了する。バッチ全体を失うため、実搭載メモリに対して余裕のある値にする。
- HEIC / HEIF は別のデコーダを使うため、この上限は適用されない。

## 処理仕様

- 出力先は既定で `<ユーザーのデスクトップ>/@StorageSlim/output`。
- 入力先は既定で `<ユーザーのデスクトップ>/@StorageSlim/input`。
- 入力先 / 出力先や処理対象以外の設定はアプリ再起動後も保持する。
- 出力時は入力フォルダからの相対パスを維持し、出力フォルダ配下へ保存する。
- 上書き禁止時に同名ファイルがある場合は連番を付与する。
- 失敗したファイルがあっても、成功分は保存して処理を継続する。
- 停止操作は現在処理中のファイル完了後に反映する。エンコード中のファイルを途中破棄しないための仕様。
- エンコードはメモリ上で完了させてからファイルを書き出す。失敗時に 0 byte のファイルを残さないための仕様。
- アニメーション GIF はフレームをキャンバスへ合成してからリサイズし、書き出す際は直前フレームとの差分矩形へ戻す。GIF のフレームは部分矩形で格納されうるため、フレーム単体をリサイズすると位置とサイズが破綻するため。
- 再圧縮した結果が元より大きく、かつコピーで指示を満たせる場合は元ファイルをコピーする。形式変換・リサイズ・メタデータ削除のいずれかを指定している場合は、指示を優先して再圧縮結果を出力する。

## 自分でビルドして使う

StorageSlim は MIT ライセンスの OSS です。以下の手順でビルドすれば、配布されているものと同じアプリを無償で入手できます。

必要なもの:

- Node.js 20 以降 / npm
- Rust / Cargo（rustup 経由での導入を推奨）
- Windows: Visual Studio Build Tools の「C++ によるデスクトップ開発」ワークロード、および WebView2 ランタイム（Windows 11 には標準で入っている）
- macOS: Xcode Command Line Tools

手順（Windows は PowerShell、macOS は Terminal.app などのシェル）:

```powershell
git clone https://github.com/<owner>/StorageSlim.git
cd StorageSlim
npm install
npm run tauri build
```

生成物:

```text
<repo>/src-tauri/target/release/storageslim.exe
<repo>/src-tauri/target/release/bundle/msi/StorageSlim_<version>_x64_en-US.msi
<repo>/src-tauri/target/release/bundle/nsis/StorageSlim_<version>_x64-setup.exe
```

### 注意: `cargo build` ではアプリになりません

`src-tauri` で `cargo build` を実行すると exe 自体は生成されますが、フロントエンドのビルド成果物（`dist/`）が埋め込まれないため、起動しても画面が表示されません。

ビルドは必ず Tauri CLI（`npm run tauri build`）を使ってください。Tauri CLI は `beforeBuildCommand` として `npm run build` を先に実行し、生成された `dist/` を exe へ埋め込みます。`cargo build` はこの手順を踏まないため、単体では成立しません。

### その他の注意

- 初回ビルドは依存クレート（rav1e などの AVIF エンコーダを含む）のコンパイルに時間がかかります。マシンによっては 10〜30 分程度かかりますが、2 回目以降はキャッシュが効きます。
- ビルド済みファイルには署名を付けていないため、Windows では初回起動時に SmartScreen の警告が表示されます。`詳細情報` → `実行` で起動できます。

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
- 圧縮の結果、出力が元より大きくなることがある（PNG が得意な素材を JPEG へ変換した場合など）。その場合は結果一覧の SAVED 列に `+◯◯ KB / +◯◯%` と増加量を表示する。
- 形式・寸法・メタデータのいずれも変えない指定（オリジナル維持 / リサイズなし / メタデータ保持）で、再圧縮すると元より大きくなる場合は、元ファイルをコピーする。この場合は `再圧縮すると大きくなるため元ファイルをコピー` を警告として表示する。
- 動作確認用のサンプル画像と手順は `samples/` および `docs/verification-guide.md` を参照。

## ライセンス

- 本体: MIT License。全文は [LICENSE](LICENSE) を参照。
- 同梱しているオープンソースソフトウェアの著作権表示とライセンス条文は [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) にまとめている。

サードパーティ表記は依存グラフから自動生成する。依存を追加・更新したら再生成してコミットすること。

```powershell
npm run notices
```

再配布時は `LICENSE` と `THIRD-PARTY-NOTICES.md` を必ず同梱する。msi / nsis / app などのバンドルへは `src-tauri/tauri.conf.json` の `bundle.resources` 経由で自動的に含まれる。ポータブル ZIP を手で作る場合は、この 2 ファイルを自分で入れること。
