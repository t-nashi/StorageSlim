// LGPL 構成の FFmpeg / ffprobe を src-tauri/binaries/ へ配置する。
//
// GPL ビルド（libx264 / libx265 を含むもの）は同梱しない。
// 配布物のライセンスを MIT のまま保つための判断（docs/decision-log.md の D-18）。
//
// 使い方: node scripts/fetch-ffmpeg.mjs
//
// 取得したバイナリは git 管理対象外。配布時は THIRD-PARTY-NOTICES.md へ
// バージョン・configure オプション・同一リビジョンのソース入手先を記載すること。

import { execFileSync } from "node:child_process";
import { mkdirSync, existsSync, writeFileSync, renameSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const DEST_DIR = join(process.cwd(), "src-tauri", "binaries");

/** Tauri の externalBin はターゲットトリプル付きの名前を要求する。 */
const TARGETS = {
  win32: {
    triple: "x86_64-pc-windows-msvc",
    exe: ".exe",
    // BtbN のリリースには gpl 版と lgpl 版がある。必ず lgpl 版を使う。
    url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-lgpl.zip",
  },
};

function fail(message) {
  console.error(`\n[fetch-ffmpeg] ${message}\n`);
  process.exit(1);
}

const target = TARGETS[process.platform];
if (!target) {
  fail(
    [
      `${process.platform} 向けの LGPL 構成の配布ビルドは用意されていません。`,
      "macOS は自前ビルドが必要です。目安となる configure:",
      "",
      "  ./configure --prefix=... \\",
      "    --disable-gpl --disable-nonfree --disable-doc --disable-debug \\",
      "    --enable-shared=no --enable-static \\",
      "    --enable-videotoolbox --enable-audiotoolbox \\",
      "    --enable-libsvtav1 --enable-libvpx --enable-libopus",
      "",
      "ビルドした ffmpeg / ffprobe を src-tauri/binaries/ へ",
      "ffmpeg-aarch64-apple-darwin のような名前で置いてください。",
    ].join("\n"),
  );
}

mkdirSync(DEST_DIR, { recursive: true });

const work = join(tmpdir(), `storageslim-ffmpeg-${process.pid}`);
mkdirSync(work, { recursive: true });
const archive = join(work, "ffmpeg-lgpl.zip");

console.log(`[fetch-ffmpeg] downloading ${target.url}`);
const response = await fetch(target.url, { redirect: "follow" });
if (!response.ok) {
  fail(`ダウンロードに失敗しました: HTTP ${response.status}`);
}
writeFileSync(archive, Buffer.from(await response.arrayBuffer()));

console.log("[fetch-ffmpeg] extracting");
if (process.platform === "win32") {
  execFileSync(
    "powershell",
    ["-NoProfile", "-Command", `Expand-Archive -LiteralPath '${archive}' -DestinationPath '${work}' -Force`],
    { stdio: "inherit" },
  );
} else {
  execFileSync("unzip", ["-q", "-o", archive, "-d", work], { stdio: "inherit" });
}

// 展開先は ffmpeg-master-latest-win64-lgpl/bin/ の下。
const { readdirSync } = await import("node:fs");
const extracted = readdirSync(work).find((name) => name.startsWith("ffmpeg-") && !name.endsWith(".zip"));
if (!extracted) {
  fail("展開結果が見つかりませんでした。");
}
const binDir = join(work, extracted, "bin");

for (const tool of ["ffmpeg", "ffprobe"]) {
  const from = join(binDir, `${tool}${target.exe}`);
  if (!existsSync(from)) {
    fail(`${from} が見つかりません。`);
  }
  const to = join(DEST_DIR, `${tool}-${target.triple}${target.exe}`);
  renameSync(from, to);
  console.log(`[fetch-ffmpeg] placed ${to}`);
}

// ライセンス表記に必要な情報を残す。
const version = execFileSync(join(DEST_DIR, `ffmpeg-${target.triple}${target.exe}`), ["-version"], {
  encoding: "utf8",
});
writeFileSync(join(DEST_DIR, "FFMPEG-BUILD-INFO.txt"), `source: ${target.url}\n\n${version}`);
console.log("[fetch-ffmpeg] wrote FFMPEG-BUILD-INFO.txt");

rmSync(work, { recursive: true, force: true });

if (version.includes("--enable-gpl")) {
  fail(
    "取得したビルドが GPL 構成です。配布物へ同梱してはいけません。lgpl 版の URL を確認してください。",
  );
}

console.log("[fetch-ffmpeg] done");
