# StorageSlim リリース手順

最終更新: 2026-09-12
ステータス: 運用中

## 1. 目的

BOOTH で頒布しているビルド済みバイナリを差し替えるまでの手順を、取りこぼしなく実行するためのチェックリストです。

バージョン番号がリポジトリ内の複数箇所に散っているため、1 箇所でも漏れるとインストーラのファイル名と同梱ドキュメントの記載が食い違います。毎回このファイルを上から順にたどってください。

- BOOTH 商品: https://t-nashi.booth.pm/items/8783008
- 頒布物の基準はタグです。master を直接ビルドしたものは頒布しません

## 2. 前提

- Windows 版と macOS 版は別マシンでビルドします。クロスビルドはしません
- 署名は付けていません。初回起動時の OS 警告は仕様として `docs/buyer-readme.txt` で案内します
- BOOTH は `exe` / `dmg` / `txt` を単体でアップロードできません。必ず ZIP にまとめます

## 3. リリース前の確認

- [ ] master が意図した状態になっている（マージ漏れ・未マージのブランチがない）
- [ ] `npm run build` が通る
- [ ] `cd src-tauri && cargo test` が通る
- [ ] `samples/` を使った手動確認を済ませた（手順は `docs/verification-guide.md`）
- [ ] 依存クレート / npm パッケージを追加・更新した場合は `npm run notices` で `THIRD-PARTY-NOTICES.md` を再生成した

## 4. バージョン番号の更新

次の 4 箇所を同じ番号に揃えます。**1 箇所でも漏れると成果物のファイル名とドキュメントが食い違います。**

- [ ] `package.json` の `version`
- [ ] `src-tauri/Cargo.toml` の `version`
- [ ] `src-tauri/tauri.conf.json` の `version`
- [ ] `src-tauri/Cargo.lock` の `storageslim` エントリ（`cargo check` を 1 回走らせれば追随します）

続けて、バージョン番号が本文に直書きされている箇所を直します。

- [ ] `docs/buyer-readme.txt` の「2. インストール」にある `StorageSlim_<version>_x64-setup.exe` と `StorageSlim_<version>_aarch64.dmg`
- [ ] `README.md` の「ビルド済みバイナリの頒布について」にあるタグ名

確認コマンド:

```bash
grep -rn "0\.1\.0" README.md docs/buyer-readme.txt package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
```

## 5. タグ付け

- [ ] バージョン更新をコミットする
- [ ] `git tag v<version>` を打つ
- [ ] `git push origin master --tags`

ビルドは**必ずタグの状態から**行います。タグ以後に master へコミットしてしまった場合は、`git checkout v<version>` してからビルドし直してください。

## 6. ビルド

### 共通

```bash
npm install
npm run ffmpeg
npm run tauri build
```

`npm run ffmpeg` は同梱する ffmpeg / ffprobe を用意します。macOS ではソースからビルドするため 15 分程度かかります。`cargo build` ではフロントエンドが埋め込まれないため、必ず `npm run tauri build` を使います。

### 成果物

```text
# Windows
<repo>/src-tauri/target/release/bundle/nsis/StorageSlim_<version>_x64-setup.exe

# macOS
<repo>/src-tauri/target/release/bundle/dmg/StorageSlim_<version>_aarch64.dmg
```

- [ ] Windows 版をビルドし、インストールして起動を確認した
- [ ] macOS 版をビルドし、インストールして起動を確認した
- [ ] 両方で画像圧縮と動画圧縮を 1 件ずつ実行した

## 7. 頒布用 ZIP の作成

ZIP は Python で作ります。Finder や右クリックの圧縮を使うと `__MACOSX` や `.DS_Store` が入ります。

同梱するもの:

- インストーラ本体（Windows は `.exe`、macOS は `.dmg`）
- `docs/buyer-readme.txt`（`はじめにお読みください.txt` として入れる）
- `LICENSE`
- `THIRD-PARTY-NOTICES.md`

`LICENSE` と `THIRD-PARTY-NOTICES.md` はインストーラ内部には `bundle.resources` 経由で入りますが、**ZIP の直下にも置きます**。購入者が展開した時点で読めるようにするためです。

- [ ] Windows 版の ZIP を作成した
- [ ] macOS 版の ZIP を作成した
- [ ] 展開して中身とファイル名を確認した

## 8. BOOTH の更新

- [ ] 旧バージョンのファイルを削除し、新しい ZIP をアップロードした
- [ ] 商品説明の動作環境・機能一覧を更新した
- [ ] 商品説明に更新履歴（バージョン、日付、変更点）を追記した
- [ ] 機能を追加した場合は商品画像を撮り直した

BOOTH には購入者への自動通知がありません。既存の購入者は購入履歴から再ダウンロードすれば新しいものを取得できますが、更新されたことに気づく手段が商品説明しかないため、履歴の記載を省略しないでください。

## 9. リリース後

- [ ] `README.md` の頒布の節にある「機能差の注記」が、現在の状況と合っているか確認する
- [ ] 頒布 ZIP と商品画像をバックアップ先へ保存した

## 10. 頒布物と master が食い違う期間について

master に機能を入れてから BOOTH を差し替えるまでの間は、README に載っている機能が頒布物に入っていない状態になります。GitHub 上で見える README は master のものなので、購入検討者はこの差分に気づけません。

このため README には、頒布物がタグ時点のものであることを常に明記しておきます。差し替えを頻繁に行わない方針を取る場合は、この期間が長くなることを前提に、商品説明側にも同じ注記を置いてください。
