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
    // 指定しないとビルドしたマシンの OS バージョンが最低要件になり、
    // 古い macOS では ffmpeg だけが起動しない状態で配布してしまう。
    // Apple Silicon は macOS 11 以降なので、そこへ合わせる。
    deploymentTarget: "11.0",
  },
};

/**
 * FFmpeg へ静的リンクする依存。Homebrew のものはビルドしたマシンの OS
 * バージョンが最低要件として焼き込まれているため使えない。同じ理由で
 * これらもソースからビルドする。どちらも BSD ライセンス。
 */
const MACOS_DEPS = {
  libvpx: {
    version: "1.16.0",
    url: "https://github.com/webmproject/libvpx/archive/refs/tags/v1.16.0.tar.gz",
    sha256: "7a479a3c66b9f5d5542a4c6a1b7d3768a983b1e5c14c60a9396edc9b649e015c",
    dir: "libvpx-1.16.0",
    licenseFile: "LICENSE",
  },
  opus: {
    version: "1.6.1",
    url: "https://downloads.xiph.org/releases/opus/opus-1.6.1.tar.gz",
    sha256: "6ffcb593207be92584df15b32466ed64bbec99109f007c82205f0194572411a1",
    dir: "opus-1.6.1",
    licenseFile: "COPYING",
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
 * 固定バージョンのソースを取得し、sha256 を検証して展開する。
 */
function downloadAndExtract({ url, sha256, dir }) {
  const archive = join(work, url.split("/").pop());
  console.log(`[fetch-ffmpeg] downloading ${url}`);
  execFileSync("curl", ["-fsSL", "-o", archive, url], { stdio: "inherit" });

  const digest = execFileSync("shasum", ["-a", "256", archive], { encoding: "utf8" }).split(/\s+/)[0];
  if (digest !== sha256) {
    fail(`${url} の sha256 が想定と違います。\n  期待: ${sha256}\n  実際: ${digest}`);
  }

  execFileSync("tar", ["xf", archive, "-C", work], { stdio: "inherit" });
  const extracted = join(work, dir);
  if (!existsSync(extracted)) {
    fail(`${extracted} が見つかりません。展開結果を確認してください。`);
  }
  return extracted;
}

/**
 * 最低動作 OS バージョンを検査する。
 *
 * 指定を忘れるとビルドしたマシンの OS バージョンが焼き込まれ、
 * 古い macOS では「アプリは起動するが動画モードだけ動かない」という
 * 分かりにくい壊れ方をする。ビルド時点で止める。
 */
function assertDeploymentTarget(path, label) {
  const loadCommands = execFileSync("otool", ["-l", path], { encoding: "utf8" });
  const versions = [...loadCommands.matchAll(/^\s*minos\s+(\S+)/gm)].map((match) => match[1]);
  const unexpected = [...new Set(versions)].filter((version) => version !== target.deploymentTarget);
  if (versions.length === 0 || unexpected.length > 0) {
    fail(
      [
        `${label} の最低動作 OS バージョンが想定と違います。`,
        `  期待: ${target.deploymentTarget}`,
        `  実際: ${versions.length === 0 ? "(取得できず)" : [...new Set(versions)].join(", ")}`,
        "MACOSX_DEPLOYMENT_TARGET が効いているか確認してください。",
      ].join("\n"),
    );
  }
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

  const depsPrefix = join(work, "deps");
  const jobs = execFileSync("sysctl", ["-n", "hw.ncpu"], { encoding: "utf8" }).trim();
  const buildEnv = { ...process.env, MACOSX_DEPLOYMENT_TARGET: target.deploymentTarget };

  const libvpxSource = downloadAndExtract(MACOS_DEPS.libvpx);
  console.log("[fetch-ffmpeg] building libvpx");
  const libvpxBuild = join(work, "libvpx-build");
  mkdirSync(libvpxBuild, { recursive: true });
  // libvpx のターゲット名は darwin20 が macOS 11 に対応する。
  execFileSync(
    join(libvpxSource, "configure"),
    [
      "--target=arm64-darwin20-gcc",
      `--prefix=${depsPrefix}`,
      "--disable-examples",
      "--disable-tools",
      "--disable-docs",
      "--disable-unit-tests",
      "--enable-static",
      "--disable-shared",
      "--enable-pic",
      "--enable-vp9",
      "--disable-vp8",
    ],
    { cwd: libvpxBuild, stdio: "inherit", env: buildEnv },
  );
  execFileSync("make", [`-j${jobs}`], { cwd: libvpxBuild, stdio: "inherit", env: buildEnv });
  execFileSync("make", ["install"], { cwd: libvpxBuild, stdio: "inherit", env: buildEnv });

  const opusSource = downloadAndExtract(MACOS_DEPS.opus);
  console.log("[fetch-ffmpeg] building opus");
  execFileSync(
    "./configure",
    [`--prefix=${depsPrefix}`, "--disable-shared", "--enable-static", "--disable-doc", "--disable-extra-programs"],
    { cwd: opusSource, stdio: "inherit", env: buildEnv },
  );
  execFileSync("make", [`-j${jobs}`], { cwd: opusSource, stdio: "inherit", env: buildEnv });
  execFileSync("make", ["install"], { cwd: opusSource, stdio: "inherit", env: buildEnv });

  for (const [name, dep] of Object.entries(MACOS_DEPS)) {
    const built = join(depsPrefix, "lib", name === "opus" ? "libopus.a" : "libvpx.a");
    assertDeploymentTarget(built, `${name} ${dep.version}`);
  }

  const sourceDir = downloadAndExtract({
    url: target.url,
    sha256: target.sha256,
    dir: `ffmpeg-${target.version}`,
  });

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
    `--extra-cflags=-I${join(depsPrefix, "include")}`,
    `--extra-ldflags=-L${join(depsPrefix, "lib")}`,
  ];

  console.log("[fetch-ffmpeg] configuring ffmpeg");
  execFileSync("./configure", configureArgs, {
    cwd: sourceDir,
    stdio: "inherit",
    env: { ...buildEnv, PKG_CONFIG_PATH: join(depsPrefix, "lib", "pkgconfig") },
  });

  console.log(`[fetch-ffmpeg] building ffmpeg (make -j${jobs})。10 分ほどかかります`);
  execFileSync("make", [`-j${jobs}`], { cwd: sourceDir, stdio: "inherit", env: buildEnv });
  execFileSync("make", ["install"], { cwd: sourceDir, stdio: "inherit", env: buildEnv });

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
    assertDeploymentTarget(to, tool);
    console.log(`[fetch-ffmpeg] placed ${to}`);
  }

  copyFileSync(join(sourceDir, "COPYING.LGPLv3"), join(DEST_DIR, "FFMPEG-LICENSE.txt"));
  console.log("[fetch-ffmpeg] placed FFMPEG-LICENSE.txt");
  writeGplV3FromSource(sourceDir);

  // 静的リンクしたライブラリの条文も配布物へ入れる。どちらも BSD 系で、
  // バイナリ配布時に著作権表示とライセンス文の同梱を求めている。
  const staticNotices = [
    { name: `libvpx ${MACOS_DEPS.libvpx.version}`, path: join(libvpxSource, MACOS_DEPS.libvpx.licenseFile) },
    { name: `opus ${MACOS_DEPS.opus.version}`, path: join(opusSource, MACOS_DEPS.opus.licenseFile) },
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

  return [
    `source: ${target.url}`,
    `sha256: ${target.sha256}`,
    `deployment target: macOS ${target.deploymentTarget}`,
    "",
    "statically linked:",
    ...Object.entries(MACOS_DEPS).map(([name, dep]) => `  ${name} ${dep.version}: ${dep.url} (sha256 ${dep.sha256})`),
  ].join("\n");
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
