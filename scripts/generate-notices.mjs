// 配布物に同梱するサードパーティライセンス表記を生成する。
//
//   node scripts/generate-notices.mjs
//
// Rust 側は cargo metadata の解決グラフから、npm 側は `npm ls --omit=dev` から、
// 実際に配布物へ入る依存だけを集めて THIRD-PARTY-NOTICES.md を書き出す。
// build-dependencies / devDependencies はビルド時にしか使われず配布物に含まれないため除外する。

import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputPath = join(repoRoot, "THIRD-PARTY-NOTICES.md");

// libwebp や rav1e のように C ソースや別プロジェクトを同梱する crate があるため、
// crate 直下だけでなく数階層下のライセンスファイルも拾う。
const LICENSE_FILE = /^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT|PATENTS|AUTHORS)([-._].*)?$/i;
const MAX_DEPTH = 3;
const SKIP_DIRS = new Set(["node_modules", "target", ".git", "tests", "benches", "examples"]);
const MAX_LICENSE_BYTES = 512 * 1024;

function run(command, args, cwd) {
  // Windows では npm の実体が npm.cmd で、Node 24 以降は shell 経由でないと起動できない。
  // 引数はこのファイル内の固定値のみなので、shell 経由でも注入の余地はない。
  const useShell = process.platform === "win32" && command === "npm";
  return execFileSync(command, args, { cwd, encoding: "utf8", maxBuffer: 256 * 1024 * 1024, shell: useShell });
}

function findLicenseFiles(root) {
  const found = [];
  const walk = (dir, depth) => {
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (depth < MAX_DEPTH && !SKIP_DIRS.has(entry.name)) walk(full, depth + 1);
      } else if (LICENSE_FILE.test(entry.name)) {
        found.push(full);
      }
    }
  };
  walk(root, 1);
  return found.sort();
}

function readLicenseTexts(root) {
  const texts = [];
  for (const file of findLicenseFiles(root)) {
    let body;
    try {
      if (statSync(file).size > MAX_LICENSE_BYTES) continue;
      body = readFileSync(file, "utf8").replace(/\r\n/g, "\n").trim();
    } catch {
      continue;
    }
    if (!body) continue;
    texts.push({ label: file.slice(root.length + 1).split(sep).join("/"), body });
  }
  return texts;
}

function collectRustPackages() {
  const target = process.env.NOTICES_TARGET ?? "x86_64-pc-windows-msvc";
  const meta = JSON.parse(
    run(
      "cargo",
      ["metadata", "--format-version", "1", "--offline", "--filter-platform", target],
      join(repoRoot, "src-tauri"),
    ),
  );

  const nodes = new Map(meta.resolve.nodes.map((node) => [node.id, node]));
  const packages = new Map(meta.packages.map((pkg) => [pkg.id, pkg]));
  const workspace = new Set(meta.workspace_members);

  // 通常依存だけを辿る。dep_kinds の kind が null のものが通常依存で、
  // "build" / "dev" はバイナリへリンクされない。
  const reached = new Set();
  const queue = [...workspace];
  while (queue.length > 0) {
    const node = nodes.get(queue.pop());
    if (!node) continue;
    for (const dep of node.deps) {
      const isNormal = dep.dep_kinds.some((kind) => kind.kind === null || kind.kind === undefined);
      if (!isNormal || reached.has(dep.pkg)) continue;
      reached.add(dep.pkg);
      queue.push(dep.pkg);
    }
  }

  return [...reached]
    .filter((id) => !workspace.has(id))
    .map((id) => {
      const pkg = packages.get(id);
      return {
        name: pkg.name,
        version: pkg.version,
        license: pkg.license ?? pkg.license_file ?? "(表記なし / リポジトリを参照)",
        repository: pkg.repository ?? null,
        texts: readLicenseTexts(dirname(pkg.manifest_path)),
      };
    })
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

function collectNpmPackages() {
  const tree = JSON.parse(run("npm", ["ls", "--omit=dev", "--all", "--json", "--long"], repoRoot));
  const seen = new Map();

  const walk = (deps) => {
    for (const [name, info] of Object.entries(deps ?? {})) {
      const version = info.version ?? "0.0.0";
      const key = name + "@" + version;
      if (!seen.has(key) && info.path) {
        seen.set(key, {
          name,
          version,
          license:
            typeof info.license === "string" ? info.license : (info.license?.type ?? "(表記なし / リポジトリを参照)"),
          repository:
            typeof info.repository === "string" ? info.repository : (info.repository?.url ?? null),
          texts: readLicenseTexts(info.path),
        });
      }
      walk(info.dependencies);
    }
  };
  walk(tree.dependencies);

  return [...seen.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

function renderSection(title, note, packages) {
  const lines = [
    "## " + title,
    "",
    note,
    "",
    "| パッケージ | バージョン | ライセンス | リポジトリ |",
    "| --- | --- | --- | --- |",
  ];
  for (const pkg of packages) {
    lines.push(`| ${pkg.name} | ${pkg.version} | ${pkg.license} | ${pkg.repository ?? "-"} |`);
  }
  lines.push("");
  return lines.join("\n");
}

// 依存が 300 件を超えるため、内容が同一の条文（Apache-2.0 や MIT の定型文）は
// 1 度だけ載せ、どのパッケージに適用されるかを併記する。
function groupLicenseTexts(sections) {
  const groups = new Map();
  for (const [kind, packages] of sections) {
    for (const pkg of packages) {
      for (const text of pkg.texts) {
        if (!groups.has(text.body)) groups.set(text.body, { label: text.label, body: text.body, users: [] });
        groups.get(text.body).users.push(`${kind}: ${pkg.name} ${pkg.version} (${text.label})`);
      }
    }
  }
  return [...groups.values()].sort((a, b) => b.users.length - a.users.length || a.label.localeCompare(b.label));
}

function renderLicenseTexts(groups) {
  const lines = ["## ライセンス全文", "", "内容が同一の条文は 1 度だけ掲載し、適用されるパッケージを併記しています。", ""];
  groups.forEach((group, index) => {
    lines.push(`### ${index + 1}. ${group.label}`, "");
    lines.push("適用パッケージ:", "");
    for (const user of group.users) lines.push(`- ${user}`);
    lines.push("", "```text", group.body, "```", "");
  });
  return lines.join("\n");
}

const rust = collectRustPackages();
const npm = collectNpmPackages();
const groups = groupLicenseTexts([
  ["Rust", rust],
  ["npm", npm],
]);

/**
 * 同梱する FFmpeg の表記。
 *
 * cargo / npm の依存ではないため自動収集できない。LGPL は「ライセンス文の同梱」
 * 「使用の明示」「対応ソースの提供」を求めるので、ここに固定で載せる。
 * バイナリを更新したら src-tauri/binaries/FFMPEG-BUILD-INFO.txt と突き合わせること。
 */
const ffmpegSection = [
  "## 同梱バイナリ: FFmpeg",
  "",
  "動画圧縮モードは FFmpeg を外部プロセスとして呼び出します。ライブラリとしてリンクはしていません。",
  "",
  "- 名称: FFmpeg (ffmpeg / ffprobe)",
  "- ライセンス: LGPL v3 以降（`--enable-version3` を含む構成のため。LGPL v3 は GPL v3 の条文を参照します）",
  "- GPL 構成ではありません。`libx264` / `libx265` は含みません（`--disable-libx264 --disable-libx265`）",
  "- 取得元: https://github.com/BtbN/FFmpeg-Builds （`ffmpeg-master-latest-win64-lgpl`）",
  "- 対応ソース: https://github.com/BtbN/FFmpeg-Builds および https://git.ffmpeg.org/ffmpeg.git （同梱バイナリのバージョンは配布物内の `FFMPEG-BUILD-INFO.txt` を参照）",
  "- ライセンス全文: 配布物内の `FFMPEG-LICENSE.txt`（LGPL v3）と `FFMPEG-GPL-3.0.txt`（GPL v3）",
  "- ビルド構成: 配布物内の `FFMPEG-BUILD-INFO.txt`（`ffmpeg -version` の configure 行）",
  "",
  "FFmpeg は別プロセスとして呼び出す独立した実行ファイルです。差し替えたい場合は、",
  "アプリの設定にある `ffmpeg のパス` へ任意のビルドを指定できます。",
  "",
  "StorageSlim 本体のライセンスは MIT です（`LICENSE`）。",
  "",
].join("\n");

const header = [
  "# サードパーティライセンス表記",
  "",
  "StorageSlim は以下のオープンソースソフトウェアを利用しています。",
  "各ソフトウェアの著作権は各権利者に帰属し、以下のライセンス条件のもとで再配布しています。",
  "",
  "このファイルは `node scripts/generate-notices.mjs` で自動生成しています。手で編集せず、依存を変更したら生成し直してください。",
  "",
  "- 同梱バイナリ: FFmpeg (LGPL v3)",
  `- Rust クレート: ${rust.length} 件`,
  `- npm パッケージ: ${npm.length} 件`,
  `- 収録しているライセンス条文: ${groups.length} 種`,
  "",
].join("\n");

writeFileSync(
  outputPath,
  [
    header,
    ffmpegSection,
    renderSection(
      "Rust クレート",
      "アプリ本体（バックエンド）へリンクされる依存です。build-dependencies と dev-dependencies は配布物に含まれないため除外しています。",
      rust,
    ),
    renderSection(
      "npm パッケージ",
      "フロントエンドのバンドルへ取り込まれる依存です。devDependencies は配布物に含まれないため除外しています。",
      npm,
    ),
    renderLicenseTexts(groups),
  ].join("\n"),
  "utf8",
);

console.log(`wrote ${outputPath} (rust: ${rust.length}, npm: ${npm.length}, licenses: ${groups.length})`);
