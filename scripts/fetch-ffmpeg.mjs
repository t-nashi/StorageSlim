// LGPL 構成の FFmpeg / ffprobe を src-tauri/binaries/ へ配置する。
//
// GPL ビルド（libx264 / libx265 を含むもの）は同梱しない。
// 配布物のライセンスを MIT のまま保つための判断（docs/decision-log.md の D-18）。
//
// Windows: BtbN の LGPL ビルドを取得する。
// macOS: LGPL 構成の配布ビルドが存在しないため、ソースから自前でビルドする。
//
// 使い方: node scripts/fetch-ffmpeg.mjs
//
// 取得したバイナリは git 管理対象外。配布時は THIRD-PARTY-NOTICES.md へ
// バージョン・configure オプション・同一リビジョンのソース入手先を記載すること。

import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  existsSync,
  writeFileSync,
  readFileSync,
  renameSync,
  rmSync,
  copyFileSync,
  readdirSync,
} from "node:fs";
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
  darwin: {
    triple: process.arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin",
    exe: "",
    // 配布ビルドが無いためソースから作る。バージョンは固定し、
    // 配布物のライセンス表記と一致させる。
    version: "9.0.1",
    url: "https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz",
    sha256: "cf38e0e28c7e5605942c4a77755349b0145804a397af37eb1fb4c77cb237f635",
  },
};

function fail(message) {
  console.error(`\n[fetch-ffmpeg] ${message}\n`);
  process.exit(1);
}

const target = TARGETS[process.platform];
if (!target) {
  fail(`${process.platform} 向けの手順は用意されていません。Windows と macOS のみ対応しています。`);
}

mkdirSync(DEST_DIR, { recursive: true });

const work = join(tmpdir(), `storageslim-ffmpeg-${process.pid}`);
mkdirSync(work, { recursive: true });

/** LGPL v3 は GPL v3 の条文を参照するため、両方を配布物へ入れる。 */
function writeGplV3FromSource(sourceDir) {
  copyFileSync(join(sourceDir, "COPYING.GPLv3"), join(DEST_DIR, "FFMPEG-GPL-3.0.txt"));
  console.log("[fetch-ffmpeg] placed FFMPEG-GPL-3.0.txt");
}

/**
 * Windows: BtbN の LGPL ビルドを展開して配置する。
 */
async function fetchWindows() {
  const archive = join(work, "ffmpeg-lgpl.zip");
  console.log(`[fetch-ffmpeg] downloading ${target.url}`);
  const response = await fetch(target.url, { redirect: "follow" });
  if (!response.ok) {
    fail(`ダウンロードに失敗しました: HTTP ${response.status}`);
  }
  writeFileSync(archive, Buffer.from(await response.arrayBuffer()));

  console.log("[fetch-ffmpeg] extracting");
  execFileSync(
    "powershell",
    ["-NoProfile", "-Command", `Expand-Archive -LiteralPath '${archive}' -DestinationPath '${work}' -Force`],
    { stdio: "inherit" },
  );

  // 展開先は ffmpeg-master-latest-win64-lgpl/bin/ の下。
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

  // LGPL はライセンス文の同梱を求める。アーカイブ同梱のものをそのまま置く。
  const licenseNames = readdirSync(join(work, extracted)).filter((name) => /^(LICENSE|COPYING)/i.test(name));
  if (licenseNames.length === 0) {
    fail("アーカイブにライセンス文が見つかりませんでした。同梱できないため中断します。");
  }
  for (const name of licenseNames) {
    copyFileSync(join(work, extracted, name), join(DEST_DIR, `FFMPEG-${name.toUpperCase().replace(/\.TXT$/, "")}.txt`));
    console.log(`[fetch-ffmpeg] placed FFMPEG-${name.toUpperCase().replace(/\.TXT$/, "")}.txt`);
  }

  // アーカイブには LGPL の本文しか入っていないため、GPL v3 は別途取得する。
  // gnu.org は環境によって到達できないため、FFmpeg 本体が同梱している同じ本文を使う。
  const GPL3_URL = "https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/COPYING.GPLv3";
  console.log(`[fetch-ffmpeg] downloading ${GPL3_URL}`);
  const gpl = await fetch(GPL3_URL, { redirect: "follow" });
  if (!gpl.ok) {
    fail(`GPL v3 本文の取得に失敗しました: HTTP ${gpl.status}`);
  }
  const gplText = await gpl.text();
  if (!gplText.includes("GNU GENERAL PUBLIC LICENSE")) {
    fail("GPL v3 本文の内容が想定と違います。");
  }
  writeFileSync(join(DEST_DIR, "FFMPEG-GPL-3.0.txt"), gplText);
  console.log("[fetch-ffmpeg] placed FFMPEG-GPL-3.0.txt");

  return `source: ${target.url}`;
}

/**
 * macOS: ソースから LGPL 構成でビルドする。
 *
 * H.264 と AAC は OS 内蔵の VideoToolbox / AudioToolbox を使うため追加の
 * ライブラリが要らない。WebM (VP9 + Opus) だけ libvpx と libopus が必要で、
 * どちらも BSD ライセンスなので同梱できる。購入者の環境に Homebrew は
 * 無いため、これらは静的リンクする。
 */
function buildMacos() {
  for (const tool of ["clang", "make", "pkg-config"]) {
    try {
      execFileSync("command", ["-v", tool], { shell: "/bin/sh", stdio: "ignore" });
    } catch {
      fail(`${tool} が見つかりません。Xcode Command Line Tools と Homebrew を用意してください。`);
    }
  }

  const HOMEBREW = "/opt/homebrew";
  const staticLibs = join(work, "staticlibs");
  mkdirSync(staticLibs, { recursive: true });
  for (const lib of ["libvpx.a", "libopus.a"]) {
    const from = join(HOMEBREW, "lib", lib);
    if (!existsSync(from)) {
      fail(
        [
          `${from} が見つかりません。WebM (VP9 + Opus) の出力に必要です。`,
          "次のコマンドで用意してください:",
          "",
          "  brew install libvpx opus pkg-config",
        ].join("\n"),
      );
    }
    // -L の先頭に .a だけを置いたディレクトリを渡し、dylib ではなく
    // 静的ライブラリが選ばれるようにする。
    execFileSync("ln", ["-sf", from, join(staticLibs, lib)]);
  }

  const archive = join(work, `ffmpeg-${target.version}.tar.xz`);
  console.log(`[fetch-ffmpeg] downloading ${target.url}`);
  execFileSync("curl", ["-fsSL", "-o", archive, target.url], { stdio: "inherit" });

  const digest = execFileSync("shasum", ["-a", "256", archive], { encoding: "utf8" }).split(/\s+/)[0];
  if (digest !== target.sha256) {
    fail(`ソースの sha256 が想定と違います。\n  期待: ${target.sha256}\n  実際: ${digest}`);
  }
  console.log(`[fetch-ffmpeg] sha256 ok (${digest})`);

  console.log("[fetch-ffmpeg] extracting");
  execFileSync("tar", ["xf", archive, "-C", work], { stdio: "inherit" });
  const sourceDir = join(work, `ffmpeg-${target.version}`);

  const prefix = join(work, "out");
  const configureArgs = [
    `--prefix=${prefix}`,
    // GPL のライブラリは入れない。configure は最後に License 行を出すので、
    // ビルド後に -version の configure 行でも検査する。
    "--disable-gpl",
    "--disable-nonfree",
    "--enable-version3",
    "--disable-doc",
    "--disable-debug",
    // 配布先に Homebrew は無い。自動検出でリンクされると起動しないため、
    // システム外のライブラリを引き込むものは明示的に落とす。
    // （xcb / X11 は画面キャプチャ用、lzma は一部コンテナ用で、いずれも本アプリは使わない）
    "--disable-libxcb",
    "--disable-xlib",
    "--disable-lzma",
    // 再生用のバイナリは配布しない。
    "--disable-ffplay",
    "--disable-shared",
    "--enable-static",
    "--disable-libx264",
    "--disable-libx265",
    "--enable-pthreads",
    "--enable-videotoolbox",
    "--enable-audiotoolbox",
    "--enable-libvpx",
    "--enable-libopus",
    "--pkg-config-flags=--static",
    `--extra-cflags=-I${HOMEBREW}/include`,
    `--extra-ldflags=-L${staticLibs} -L${HOMEBREW}/lib`,
  ];

  console.log("[fetch-ffmpeg] configuring");
  execFileSync("./configure", configureArgs, {
    cwd: sourceDir,
    stdio: "inherit",
    env: { ...process.env, PKG_CONFIG_PATH: join(HOMEBREW, "lib", "pkgconfig") },
  });

  const jobs = execFileSync("sysctl", ["-n", "hw.ncpu"], { encoding: "utf8" }).trim();
  console.log(`[fetch-ffmpeg] building (make -j${jobs})。10 分ほどかかります`);
  execFileSync("make", [`-j${jobs}`], { cwd: sourceDir, stdio: "inherit" });
  execFileSync("make", ["install"], { cwd: sourceDir, stdio: "inherit" });

  for (const tool of ["ffmpeg", "ffprobe"]) {
    const from = join(prefix, "bin", tool);
    if (!existsSync(from)) {
      fail(`${from} が見つかりません。`);
    }
    const to = join(DEST_DIR, `${tool}-${target.triple}`);
    renameSync(from, to);

    // Homebrew の dylib へ依存が残っていると、購入者の環境で起動しない。
    const linked = execFileSync("otool", ["-L", to], { encoding: "utf8" });
    const external = linked
      .split("\n")
      .slice(1)
      .map((line) => line.trim().split(/\s+/)[0])
      .filter((path) => path && !path.startsWith("/usr/lib/") && !path.startsWith("/System/"));
    if (external.length > 0) {
      fail(
        [
          `${tool} がシステム外のライブラリへ依存しています。配布先で起動しません。`,
          ...external.map((path) => `  ${path}`),
        ].join("\n"),
      );
    }
    console.log(`[fetch-ffmpeg] placed ${to}`);
  }

  copyFileSync(join(sourceDir, "COPYING.LGPLv3"), join(DEST_DIR, "FFMPEG-LICENSE.txt"));
  console.log("[fetch-ffmpeg] placed FFMPEG-LICENSE.txt");
  writeGplV3FromSource(sourceDir);

  // 静的リンクしたライブラリの条文も配布物へ入れる。どちらも BSD 系で、
  // バイナリ配布時に著作権表示とライセンス文の同梱を求めている。
  const staticNotices = [
    { name: "libvpx", path: join(HOMEBREW, "opt", "libvpx", "LICENSE") },
    { name: "opus", path: join(HOMEBREW, "opt", "opus", "COPYING") },
  ];
  const staticTexts = staticNotices.map(({ name, path }) => {
    if (!existsSync(path)) {
      fail(`${path} が見つかりません。${name} のライセンス文を同梱できないため中断します。`);
    }
    return [
      "================================================================",
      `${name}（FFmpeg へ静的リンク）`,
      "================================================================",
      "",
      readFileSync(path, "utf8").trimEnd(),
      "",
    ].join("\n");
  });
  writeFileSync(
    join(DEST_DIR, "FFMPEG-STATIC-LIBS-LICENSE.txt"),
    [
      "同梱の FFmpeg へ静的リンクしているライブラリのライセンス条文です。",
      "",
      ...staticTexts,
    ].join("\n"),
  );
  console.log("[fetch-ffmpeg] placed FFMPEG-STATIC-LIBS-LICENSE.txt");

  return [`source: ${target.url}`, `sha256: ${target.sha256}`].join("\n");
}

const sourceInfo = process.platform === "win32" ? await fetchWindows() : buildMacos();

// ライセンス表記に必要な情報を残す。
const version = execFileSync(join(DEST_DIR, `ffmpeg-${target.triple}${target.exe}`), ["-version"], {
  encoding: "utf8",
});
writeFileSync(join(DEST_DIR, "FFMPEG-BUILD-INFO.txt"), `${sourceInfo}\n\n${version}`);
console.log("[fetch-ffmpeg] wrote FFMPEG-BUILD-INFO.txt");

rmSync(work, { recursive: true, force: true });

if (version.includes("--enable-gpl") || version.includes("--enable-nonfree")) {
  fail(
    "ビルドが GPL / nonfree 構成です。配布物へ同梱してはいけません。取得元または configure を確認してください。",
  );
}

console.log("[fetch-ffmpeg] done");
console.log(
  "[fetch-ffmpeg] 配布時は THIRD-PARTY-NOTICES.md の FFmpeg の節を、FFMPEG-BUILD-INFO.txt の内容と合わせて確認してください。",
);
