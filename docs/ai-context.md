# AI 作業引き継ぎ

## 2026-10-04 画像の表示方向維持を実装

- 最初は改善可否の調査依頼だったが、実 JPEG 提供後にユーザーが実装と JPEG 以外の対応形式への改善を依頼した。
- 原因は EXIF の回転を画素へ適用せずにメタデータを削除していたこと。元写真は6192×4128 / Orientation=8、旧出力は1500×1000 / Orientation なし。
- `src-tauri/src/lib.rs` で、JPEG / PNG / WebP / PSD の EXIF の回転・鏡像を、削除設定でもリサイズ前に画素へ反映する。保持する EXIF は通常向きへ正規化し、二重回転を防ぐ。
- 入力一覧・リサイズ基準・結果一覧の寸法も表示方向へ統一した。静止画の出力実寸は処理関数から返すため、AVIF 再読込失敗でも入力寸法へ戻らない。
- HEIC / HEIF は `heif-oxide` がコンテナの回転・鏡像を適用済み。追加回転せず、合成した irot / imir 付き素材で検証した。GIF のアニメーション経路は従来どおり。AVIF 入力の一時停止も継続する。
- `src-tauri/src/orientation_tests.rs` に、8方向・両バイトオーダー・メタデータ3設定・対応入力4形式、出力5形式のリサイズ、HEIC / HEIF の回転・反転、不正 EXIF、コピー時の表示寸法を検証するテストを追加した。
- `cargo test --lib`: 66件成功、実写真テスト1件は通常実行では ignore。実写真テストは環境変数指定で別途実行し成功した。
- AVIF 出力15ケースは外部の AVIF 対応 Pillow で寸法と特徴点の画素を確認した。Rust の AVIF 入力デコーダでは検証できない。
- 実写真のメタデータ3設定すべてで1000×1500に正立。削除は向きタグなし、他2設定は Orientation=1。削除設定の生成 JPEG を画像として開いて目視も確認した。
- 実写真をリポジトリへ追加していない。検証出力は Git 対象外の `src-tauri/target/orientation-check/mode-0`（削除）、`mode-1`（撮影日のみ）、`mode-2`（保持）にある。
- 検証の再現方法は `docs/verification-guide.md`、仕様は `docs/requirements.md`、判断は `docs/decision-log.md` の D-26 に記録。README とメタデータのヘルプ文も更新した。
- Windows で実写真テストの実行中に同じテスト EXE を再リンクすると LNK1104 になる。テスト終了後に再実行して成功を確認した。
- `npm run tauri build` 成功（TypeScript / Vite、Rust release、Windows NSIS / MSI）。修正版は `src-tauri/target/release/bundle/nsis/StorageSlim_0.1.0_x64-setup.exe` と `src-tauri/target/release/bundle/msi/StorageSlim_0.1.0_x64_en-US.msi`。インストールは未実施。
- 同日、ユーザーが起動中の修正版アプリで生成した WebP の削除／保持2設定を提供した。両方1000×1500で正立し、復号した RGB 全画素が一致。削除設定は EXIF なし（90,142 byte）、保持設定は EXIF 49,830 byte / Orientation=1（139,998 byte）。実アプリでの JPEG → WebP の表示方向維持も確認できた。
- 2026-10-08、ユーザーの依頼で修正・テスト・仕様書を `6fdcc49`（画像変換時にメタデータ設定にかかわらず表示方向を維持する）へコミットした。共有ルール・引き継ぎ記録は別コミットとして追加する。push は未実施。
