import { useEffect, useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { readDir } from "@tauri-apps/plugin-fs";
import appIconUrl from "../assets/storageslim-icon.svg";
import { AppHeader } from "../components/AppHeader";
import type { ChoiceOption } from "../components/ChoiceGroup";
import { ImageSettingsPanel } from "../components/ImageSettingsPanel";
import { PathPickerField } from "../components/PathPickerField";
import { ProgressPanel } from "../components/ProgressPanel";
import { InlineLoading, SkippedList, TablePanel, TableScroll } from "../components/TablePanel";
import { useDropTarget } from "../hooks/useDropTarget";
import { clamp, formatBytes, formatDimension, formatSavedDelta } from "../lib/format";
import { deriveDefaultInputDir, fileNameFromPath, joinNativePath } from "../lib/paths";
import { INITIAL_PROGRESS } from "../lib/progress";
import { isResizeValueMissing, resizeModeOptions } from "../lib/settings";
import type {
  AppMode,
  BatchProgress,
  BatchSettings,
  InputEntry,
  InspectResponse,
  OutputFormat,
  ProcessResponse,
  SkippedItem,
} from "../types";
import {
  AVIF_MAX_DIMENSION,
  AVIF_VIEWER_MAX_DIMENSION,
  DECODE_LIMIT_DEFAULT_MB,
  DECODE_LIMIT_MAX_MB,
  DECODE_LIMIT_MIN_MB,
  GIF_MAX_DIMENSION,
  JPEG_MAX_DIMENSION,
  WEBP_MAX_DIMENSION,
} from "../types";

const STORAGE_KEY = "storageslim.settings.v1";
const INPUT_SOURCE_KEY = "storageslim.inputSourceDir.v1";
const imageExtensions = new Set(["gif", "jpg", "jpeg", "png", "webp", "avif", "heic", "heif"]);

const fileFilters = [
  {
    name: "Images",
    extensions: ["gif", "jpg", "jpeg", "png", "webp", "avif", "heic", "heif"],
  },
];

function hasAllowedImageExtension(path: string): boolean {
  const normalized = path.replace(/\\/g, "/");
  const extension = normalized.split(".").pop()?.toLowerCase();
  return extension ? imageExtensions.has(extension) : false;
}

const outputOptions: Array<{ value: OutputFormat; label: string }> = [
  { value: "original", label: "オリジナル維持" },
  { value: "gif", label: "GIF" },
  { value: "jpeg", label: "JPEG" },
  { value: "png", label: "PNG" },
  { value: "webp", label: "WebP" },
  { value: "avif", label: "AVIF" },
];

function createDefaultSettings(defaultOutputDir: string): BatchSettings {
  return {
    outputFormat: "original",
    outputMode: "custom",
    customOutputDir: defaultOutputDir,
    overwrite: false,
    resize: {
      mode: "none",
      value: null,
      unit: "px",
    },
    quality: {
      jpegQuality: 82,
      webpQuality: 80,
      avifQuality: 55,
      pngCompression: 6,
      gifColors: 128,
    },
    metadataMode: "strip",
    timestamps: {
      preserveCreationTime: false,
      preserveLastWriteTime: false,
    },
    decodeLimitMb: DECODE_LIMIT_DEFAULT_MB,
  };
}

async function collectImageFilesInDirectory(rootPath: string): Promise<string[]> {
  const files: string[] = [];

  async function walk(currentPath: string) {
    const entries = await readDir(currentPath);
    for (const entry of entries) {
      const nextPath = joinNativePath(currentPath, entry.name);
      if (entry.isDirectory) {
        await walk(nextPath);
        continue;
      }
      if (entry.isFile && hasAllowedImageExtension(nextPath)) {
        files.push(nextPath);
      }
    }
  }

  await walk(rootPath);
  return files.sort((left, right) => left.localeCompare(right));
}

function normalizeSettings(raw: unknown, defaultOutputDir: string): BatchSettings {
  const fallback = createDefaultSettings(defaultOutputDir);
  if (!raw || typeof raw !== "object") {
    return fallback;
  }

  const candidate = raw as Partial<BatchSettings>;
  const resize = candidate.resize ?? fallback.resize;
  const quality = candidate.quality ?? fallback.quality;
  const timestamps = candidate.timestamps ?? fallback.timestamps;

  return {
    outputFormat:
      candidate.outputFormat && outputOptions.some((option) => option.value === candidate.outputFormat)
        ? candidate.outputFormat
        : fallback.outputFormat,
    outputMode: "custom",
    customOutputDir:
      typeof candidate.customOutputDir === "string" && candidate.customOutputDir.trim().length > 0
        ? candidate.customOutputDir
        : defaultOutputDir,
    overwrite: Boolean(candidate.overwrite),
    resize: {
      mode:
        resize.mode && resizeModeOptions.some((option) => option.value === resize.mode)
          ? resize.mode
          : fallback.resize.mode,
      value:
        typeof resize.value === "number" && Number.isFinite(resize.value) && resize.value > 0
          ? Math.round(resize.value)
          : null,
      unit: resize.unit === "percent" ? "percent" : "px",
    },
    quality: {
      jpegQuality: clamp(Number(quality.jpegQuality ?? fallback.quality.jpegQuality), 1, 100),
      webpQuality: clamp(Number(quality.webpQuality ?? fallback.quality.webpQuality), 1, 100),
      avifQuality: clamp(Number(quality.avifQuality ?? fallback.quality.avifQuality), 1, 100),
      pngCompression: clamp(Number(quality.pngCompression ?? fallback.quality.pngCompression), 0, 9),
      gifColors: clamp(Number(quality.gifColors ?? fallback.quality.gifColors), 2, 256),
    },
    metadataMode:
      candidate.metadataMode === "keep" ||
      candidate.metadataMode === "dateOnly" ||
      candidate.metadataMode === "strip"
        ? candidate.metadataMode
        : fallback.metadataMode,
    timestamps: {
      preserveCreationTime: Boolean(timestamps.preserveCreationTime),
      preserveLastWriteTime: Boolean(timestamps.preserveLastWriteTime),
    },
    decodeLimitMb: clamp(
      Math.round(Number(candidate.decodeLimitMb ?? fallback.decodeLimitMb)) || fallback.decodeLimitMb,
      DECODE_LIMIT_MIN_MB,
      DECODE_LIMIT_MAX_MB,
    ),
  };
}

function loadStoredSettings(defaultOutputDir: string): BatchSettings {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (!stored) {
      return createDefaultSettings(defaultOutputDir);
    }
    return normalizeSettings(JSON.parse(stored), defaultOutputDir);
  } catch {
    return createDefaultSettings(defaultOutputDir);
  }
}

function loadStoredInputSourceDir(defaultInputDir: string): string {
  try {
    const stored = window.localStorage.getItem(INPUT_SOURCE_KEY);
    if (stored && stored.trim().length > 0) {
      return stored;
    }
  } catch {
    // Ignore malformed local state and fall back.
  }
  return defaultInputDir;
}

/** 実行前に予測できる問題。処理を待たずに入力一覧へ出す。 */
type Preflight = {
  /** danger: このままでは失敗する / warning: 出力はできるが注意が必要 */
  level: "danger" | "warning";
  /** 状態列のタグ文言。長くすると列が横に伸びるので短く保つ。 */
  label: string;
  /** ツールチップに出す詳細。 */
  detail: string;
};

/**
 * デコード後に必要となるピクセルバッファのおおよそのバイト数。
 * 実際の色形式はデコードするまで確定しないため、形式ごとの代表値で見積もる。
 * src-tauri 側の image クレート経由のデコードに対応する値。
 */
function estimatedDecodeBytes(width: number, height: number, format: InputEntry["format"]): number {
  return width * height * (format === "jpeg" ? 3 : 4);
}

const OUTPUT_DIMENSION_LIMITS: Partial<Record<OutputFormat, { label: string; limit: number }>> = {
  webp: { label: "WebP", limit: WEBP_MAX_DIMENSION },
  jpeg: { label: "JPEG", limit: JPEG_MAX_DIMENSION },
  gif: { label: "GIF", limit: GIF_MAX_DIMENSION },
  avif: { label: "AVIF", limit: AVIF_MAX_DIMENSION },
};

/**
 * 実際に書き出される形式を求める。Rust の resolve_output_format と対応。
 * HEIC / HEIF のオリジナル維持はコピー出力なので寸法制限を持たない (null)。
 */
function resolveOutputFormat(entry: InputEntry, requested: OutputFormat): OutputFormat | null {
  if (requested !== "original") {
    return requested;
  }
  switch (entry.format) {
    case "gif":
      return "gif";
    case "jpeg":
      return "jpeg";
    case "png":
      return "png";
    case "webp":
      return "webp";
    case "avif":
      return "avif";
    default:
      return null;
  }
}

/** リサイズ後の寸法。Rust の resize_target_dimensions と対応。 */
function resizeTargetDimensions(
  width: number,
  height: number,
  resize: BatchSettings["resize"],
): [number, number] {
  if (resize.mode === "none" || resize.value == null || resize.value <= 0) {
    return [width, height];
  }

  let scaled: number;
  if (resize.unit === "px") {
    scaled = resize.value;
  } else {
    const basis =
      resize.mode === "width" ? width : resize.mode === "height" ? height : Math.max(width, height);
    scaled = Math.max(1, Math.round(basis * (resize.value / 100)));
  }

  let targetWidth: number;
  let targetHeight: number;
  if (resize.mode === "width") {
    targetWidth = Math.max(1, Math.min(scaled, width));
    targetHeight = Math.max(1, Math.round(height * (targetWidth / width)));
  } else if (resize.mode === "height") {
    targetHeight = Math.max(1, Math.min(scaled, height));
    targetWidth = Math.max(1, Math.round(width * (targetHeight / height)));
  } else {
    const limited = Math.max(1, Math.min(scaled, Math.max(width, height)));
    if (width >= height) {
      targetWidth = limited;
      targetHeight = Math.max(1, Math.round(height * (targetWidth / width)));
    } else {
      targetHeight = limited;
      targetWidth = Math.max(1, Math.round(width * (targetHeight / height)));
    }
  }

  // 拡大はしない。
  if (targetWidth >= width && targetHeight >= height) {
    return [width, height];
  }
  return [targetWidth, targetHeight];
}

/**
 * 現在の設定でこの入力を処理した場合に予測される問題を列挙する。
 *
 * デコード上限・出力形式・リサイズ値のいずれを変えても即座に更新される。
 * Rust 側の inspect は設定を知らないため、この判定はここで行う。
 * 最終的な可否は Rust 側 (decode_input_image / check_encoder_dimensions) が決める。
 */
function computePreflight(entry: InputEntry, settings: BatchSettings | null): Preflight[] {
  if (!settings || entry.width == null || entry.height == null) {
    return [];
  }

  const items: Preflight[] = [];

  // デコードは形式変換より前に走るので最初に見る。
  // HEIC / HEIF は別デコーダを使うため、この上限は適用されない。
  if (entry.format !== "heic" && entry.format !== "heif") {
    const estimated = estimatedDecodeBytes(entry.width, entry.height, entry.format);
    const decodeLimitBytes = settings.decodeLimitMb * 1024 * 1024;
    if (estimated > decodeLimitBytes) {
      items.push({
        level: "danger",
        label: `デコードに約 ${formatBytes(estimated)} 必要 (上限 ${formatBytes(decodeLimitBytes)})`,
        detail: `デコードに約 ${formatBytes(estimated)} のメモリが必要で、上限 ${formatBytes(decodeLimitBytes)} を超えています。「品質調整・その他」のデコード上限を上げるか、対象を変えてください。`,
      });
    }
  }

  const outputFormat = resolveOutputFormat(entry, settings.outputFormat);
  if (!outputFormat) {
    return items;
  }

  const [width, height] = resizeTargetDimensions(entry.width, entry.height, settings.resize);
  const hardLimit = OUTPUT_DIMENSION_LIMITS[outputFormat];
  if (hardLimit && (width > hardLimit.limit || height > hardLimit.limit)) {
    items.push({
      level: "danger",
      label: `${hardLimit.label} 上限 ${hardLimit.limit} px 超`,
      detail: `${hardLimit.label} は幅・高さとも ${hardLimit.limit} px までです（出力予定 ${width} x ${height}）。リサイズすると出力できます。`,
    });
  } else if (
    outputFormat === "avif" &&
    (width > AVIF_VIEWER_MAX_DIMENSION || height > AVIF_VIEWER_MAX_DIMENSION)
  ) {
    items.push({
      level: "warning",
      label: `主要ビューア上限 ${AVIF_VIEWER_MAX_DIMENSION} px 超`,
      detail: `${width} x ${height} は主要ビューア（libavif 系）の上限 ${AVIF_VIEWER_MAX_DIMENSION} px を超えるため、出力した AVIF を開けない場合があります。`,
    });
  }

  return items;
}

function inputNameState(
  entry: InputEntry,
  preflights: Preflight[],
): "normal" | "accent" | "warning" | "danger" {
  if (preflights.some((item) => item.level === "danger")) {
    return "danger";
  }
  if (!entry.runtimeSupported || preflights.length > 0) {
    return "warning";
  }
  return entry.animated ? "accent" : "normal";
}

function resultNameState(
  result: ProcessResponse["results"][number],
  sourceEntry?: InputEntry,
): "normal" | "accent" | "warning" | "danger" {
  if (!result.success) {
    return "danger";
  }
  if (sourceEntry?.animated) {
    return "accent";
  }
  return result.warnings.length > 0 ? "warning" : "normal";
}

function outputAllowed(entry: InputEntry, output: OutputFormat): boolean {
  if (entry.animated) {
    return output === "original" || output === "gif";
  }
  return true;
}

function outputDisabledReason(entries: InputEntry[], output: OutputFormat): string | null {
  if (entries.length === 0) {
    return null;
  }
  for (const entry of entries) {
    if (!outputAllowed(entry, output)) {
      return "アニメーション GIF を含むため、この出力形式は選択できません。";
    }
  }
  return null;
}

function mergeEntries(current: InputEntry[], incoming: InputEntry[]): InputEntry[] {
  const merged = new Map<string, InputEntry>();
  for (const entry of current) {
    merged.set(entry.sourcePath, entry);
  }
  for (const entry of incoming) {
    merged.set(entry.sourcePath, entry);
  }
  return Array.from(merged.values()).sort((a, b) => a.sourcePath.localeCompare(b.sourcePath));
}

/**
 * 画像圧縮モードの画面。
 *
 * 入力一覧と結果は App が持つ。モードを切り替えても失わないようにするため
 * （`docs/decision-log.md` の `D-17`）。
 */
export function ImageMode({
  mode,
  onModeChange,
  entries,
  setEntries,
  skipped,
  setSkipped,
  results,
  setResults,
  progress,
  setProgress,
}: {
  mode: AppMode;
  onModeChange: (next: AppMode) => void;
  entries: InputEntry[];
  setEntries: Dispatch<SetStateAction<InputEntry[]>>;
  skipped: SkippedItem[];
  setSkipped: Dispatch<SetStateAction<SkippedItem[]>>;
  results: ProcessResponse["results"];
  setResults: Dispatch<SetStateAction<ProcessResponse["results"]>>;
  progress: BatchProgress;
  setProgress: Dispatch<SetStateAction<BatchProgress>>;
}) {
  const [defaultOutputDir, setDefaultOutputDir] = useState("");
  const [defaultInputDir, setDefaultInputDir] = useState("");
  const [inputSourceDir, setInputSourceDir] = useState("");
  const [settings, setSettings] = useState<BatchSettings | null>(null);
  const [busy, setBusy] = useState(false);
  const [paused, setPaused] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [inputLoading, setInputLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  // tauri.conf.json の version を表示する。不具合報告時にビルドを特定できるようにするため。
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const dropActive = useDropTarget(addPaths);

  useEffect(() => {
    let active = true;

    async function hydrate() {
      try {
        const outputDir = await invoke<string>("get_default_output_dir");
        if (!active) {
          return;
        }
        const inputDir = deriveDefaultInputDir(outputDir);
        setDefaultOutputDir(outputDir);
        setDefaultInputDir(inputDir);
        setInputSourceDir(loadStoredInputSourceDir(inputDir));
        setSettings(loadStoredSettings(outputDir));
      } catch (error) {
        if (!active) {
          return;
        }
        const fallbackOutput = "Desktop/@StorageSlim/output";
        const fallbackInput = deriveDefaultInputDir(fallbackOutput);
        setDefaultOutputDir(fallbackOutput);
        setDefaultInputDir(fallbackInput);
        setInputSourceDir(loadStoredInputSourceDir(fallbackInput));
        setSettings(loadStoredSettings(fallbackOutput));
        setErrorMessage(error instanceof Error ? error.message : String(error));
      }
    }

    void hydrate();

    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let active = true;
    let unlistenProgress: (() => void) | undefined;

    async function bind() {
      try {
        const webview = getCurrentWebview();
        unlistenProgress = await webview.listen<BatchProgress>("batch-progress", (event) => {
          if (active) {
            setProgress(event.payload);
            if (event.payload.state === "paused") {
              setPaused(true);
            }
            if (event.payload.state === "running") {
              setPaused(false);
            }
            if (event.payload.state === "stopping") {
              setStopping(true);
              setPaused(false);
            }
          }
        });
      } catch (error) {
        console.warn("StorageSlim: Tauri progress binding is unavailable in this environment.", error);
      }
    }

    void bind();

    return () => {
      active = false;
      unlistenProgress?.();
    };
  }, []);

  useEffect(() => {
    if (!settings) {
      return;
    }
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  }, [settings]);

  useEffect(() => {
    if (!inputSourceDir) {
      return;
    }
    window.localStorage.setItem(INPUT_SOURCE_KEY, inputSourceDir);
  }, [inputSourceDir]);

  const allowedOutputs = useMemo(() => {
    const map = new Map<OutputFormat, string | null>();
    for (const option of outputOptions) {
      map.set(option.value, outputDisabledReason(entries, option.value));
    }
    return map;
  }, [entries]);

  useEffect(() => {
    if (!settings) {
      return;
    }
    const reason = allowedOutputs.get(settings.outputFormat);
    if (!reason) {
      return;
    }
    setSettings((current) => (current ? { ...current, outputFormat: "original" } : current));
  }, [allowedOutputs, settings]);

  useEffect(() => {
    let active = true;
    getVersion()
      .then((version) => {
        if (active) {
          setAppVersion(version);
        }
      })
      .catch(() => {
        // 取得できなくても表示を省くだけで、機能には影響しない。
      });
    return () => {
      active = false;
    };
  }, []);

  const totalSaved = useMemo(() => {
    return results.filter((result) => result.success).reduce((sum, result) => sum + (result.savedSize ?? 0), 0);
  }, [results]);
  const totalSavedPercent = useMemo(() => {
    const totalOriginal = results
      .filter((result) => result.success)
      .reduce((sum, result) => sum + (result.originalSize ?? 0), 0);
    if (totalOriginal <= 0) {
      return null;
    }
    return (totalSaved / totalOriginal) * 100;
  }, [results, totalSaved]);
  const failedCount = useMemo(() => {
    return results.filter((result) => !result.success).length;
  }, [results]);

  const entryBySourcePath = useMemo(() => {
    const map = new Map<string, InputEntry>();
    for (const entry of entries) {
      map.set(entry.sourcePath, entry);
    }
    return map;
  }, [entries]);

  const outputFormatChoices = useMemo<Array<ChoiceOption<OutputFormat>>>(
    () =>
      outputOptions.map((option) => ({
        value: option.value,
        label: option.label,
        disabled: Boolean(allowedOutputs.get(option.value)),
        title: allowedOutputs.get(option.value) ?? undefined,
      })),
    [allowedOutputs],
  );

  const resizeValueMissing = settings ? isResizeValueMissing(settings.resize) : true;
  const canRunBatch = entries.length > 0 && !busy && !inputLoading && !resizeValueMissing;

  async function addPaths(paths: string[]) {
    if (paths.length === 0) {
      return;
    }
    try {
      const response = await invoke<InspectResponse>("inspect_inputs", { paths });
      setEntries((current) => mergeEntries(current, response.entries));
      setSkipped((current) => current.concat(response.skipped));
      setErrorMessage(null);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function loadInputSourceDir() {
    if (inputLoading) {
      return;
    }
    const path = inputSourceDir.trim();
    if (!path) {
      setErrorMessage("入力先フォルダのパスを指定してください。");
      return;
    }
    setInputLoading(true);
    try {
      const files = await collectImageFilesInDirectory(path);
      if (files.length === 0) {
        setErrorMessage("入力先フォルダ内に対応画像が見つかりませんでした。");
        return;
      }
      await addPaths(files);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setInputLoading(false);
    }
  }

  async function pickFiles() {
    const selected = await open({
      multiple: true,
      filters: fileFilters,
    });
    if (!selected) {
      return;
    }
    await addPaths(Array.isArray(selected) ? selected : [selected]);
  }

  async function pickFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
    });
    if (!selected || Array.isArray(selected)) {
      return;
    }
    await addPaths([selected]);
  }

  async function pickInputFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: inputSourceDir || defaultInputDir,
    });
    if (!selected || Array.isArray(selected)) {
      return;
    }
    setInputSourceDir(selected);
  }

  async function pickOutputFolder() {
    if (!settings) {
      return;
    }
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: settings.customOutputDir ?? defaultOutputDir,
    });
    if (!selected || Array.isArray(selected)) {
      return;
    }
    setSettings((current) =>
      current
        ? {
            ...current,
            outputMode: "custom",
            customOutputDir: selected,
          }
        : current,
    );
  }

  async function runBatch() {
    if (!settings || entries.length === 0 || busy) {
      return;
    }
    if (settings.resize.mode !== "none" && (settings.resize.value == null || settings.resize.value <= 0)) {
      setErrorMessage("リサイズ基準を選択した場合はリサイズ値の入力が必要です。");
      return;
    }
    setBusy(true);
    setPaused(false);
    setStopping(false);
    setResults([]);
    setErrorMessage(null);
    setProgress({
      completed: 0,
      total: entries.length,
      currentPath: null,
      state: "running",
    });
    try {
      const response = await invoke<ProcessResponse>("process_batch", {
        request: {
          entries,
          settings,
        },
      });
      setResults(response.results);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
      setPaused(false);
      setStopping(false);
      setProgress((current) => ({
        ...current,
        currentPath: null,
      }));
    }
  }

  /**
   * 入力一覧を起動直後の状態へ戻す。
   * 「読み込めなかった項目」も入力の読込結果なので一緒に消す。
   * これを残すと、入力 0 件でボタンが無効になり消す手段がなくなる。
   */
  function clearInputs() {
    setEntries([]);
    setSkipped([]);
  }

  /** 結果一覧と、それに紐づく進捗表示をまとめて起動直後の状態へ戻す。 */
  function clearResults() {
    setResults([]);
    setProgress({ ...INITIAL_PROGRESS });
  }

  async function pauseBatch() {
    if (!busy || paused || stopping) {
      return;
    }
    setPaused(true);
    try {
      await invoke("pause_batch");
    } catch (error) {
      setPaused(false);
      setErrorMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function resumeBatch() {
    if (!busy || !paused || stopping) {
      return;
    }
    setPaused(false);
    try {
      await invoke("resume_batch");
    } catch (error) {
      setPaused(true);
      setErrorMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function stopBatch() {
    if (!busy || stopping) {
      return;
    }
    setStopping(true);
    setPaused(false);
    try {
      await invoke("stop_batch");
    } catch (error) {
      setStopping(false);
      setErrorMessage(error instanceof Error ? error.message : String(error));
    }
  }

  function updateSettings(updater: (current: BatchSettings) => BatchSettings) {
    setSettings((current) => (current ? updater(current) : current));
  }

  function resetInputPath() {
    setInputSourceDir(defaultInputDir);
  }

  function resetOutputPath() {
    if (!defaultOutputDir) {
      return;
    }
    updateSettings((current) => ({
      ...current,
      outputMode: "custom",
      customOutputDir: defaultOutputDir,
    }));
  }

  function resetAllSettings() {
    if (!defaultOutputDir) {
      return;
    }
    if (!window.confirm("設定を初期化します。よろしいですか？")) {
      return;
    }
    setSettings(createDefaultSettings(defaultOutputDir));
    setInputSourceDir(defaultInputDir);
  }

  if (!settings) {
    return (
      <main className="app-shell">
        <section className="panel loading-panel">
          <p className="eyebrow">StorageSlim</p>
          <h1>設定を読み込んでいます...</h1>
        </section>
      </main>
    );
  }

  const currentProgressName = fileNameFromPath(progress.currentPath);
  const pausedEffective = paused || progress.state === "paused";
  const stoppingEffective = stopping || progress.state === "stopping";
  const progressLabel = busy
    ? stoppingEffective
      ? "停止中: 現在のファイル完了後に停止します"
      : pausedEffective
        ? "一時停止中"
        : currentProgressName
          ? `処理中: ${currentProgressName}`
          : "処理を開始しています"
    : "待機中";

  return (
    <main
      className={`app-shell ${busy ? "is-processing" : ""} ${pausedEffective ? "is-paused" : ""} ${
        stoppingEffective ? "is-stopping" : ""
      }`}
    >
      <AppHeader
        iconUrl={appIconUrl}
        tagline="画像最適化ワークベンチ"
        version={appVersion}
        mode={mode}
        onModeChange={onModeChange}
        switchDisabled={busy || inputLoading}
      />

      <section className="app-grid">
        <ImageSettingsPanel
          settings={settings}
          updateSettings={updateSettings}
          outputFormatChoices={outputFormatChoices}
          onResetAll={resetAllSettings}
          clearResizeError={() =>
            setErrorMessage((current) =>
              current === "リサイズ基準を選択した場合はリサイズ値の入力が必要です。" ? null : current,
            )
          }
        />

        <section className={`panel workspace-panel ${dropActive ? "drop-active" : ""}`}>
          <div className="workspace-header">
            <div className="workspace-heading">
              <h2>入力と結果</h2>
              <p className="title-note">ファイル / フォルダはこの画面へドラッグ&ドロップでも追加できます</p>
            </div>
            <div className="run-panel">
              <ProgressPanel
                completed={progress.completed}
                total={progress.total}
                failedCount={failedCount}
                label={progressLabel}
                currentPath={progress.currentPath}
              />
              <div className="run-actions">
                {busy ? (
                  <>
                    <button
                      type="button"
                      className="ghost run-control"
                      disabled={stoppingEffective}
                      onClick={pausedEffective ? resumeBatch : pauseBatch}
                    >
                      {pausedEffective ? "再開" : "一時停止"}
                    </button>
                    <button
                      type="button"
                      className="danger run-control"
                      disabled={stoppingEffective}
                      onClick={stopBatch}
                    >
                      {stoppingEffective ? "停止中..." : "停止"}
                    </button>
                  </>
                ) : (
                  <button type="button" className="primary run-button" disabled={!canRunBatch} onClick={runBatch}>
                    最適化を実行
                  </button>
                )}
              </div>
            </div>
          </div>

          <div className="path-grid">
            <PathPickerField
              label="入力先"
              value={inputSourceDir}
              placeholder="入力先フォルダ"
              onChange={setInputSourceDir}
              onBrowse={pickInputFolder}
              onReset={resetInputPath}
              load={{ disabled: inputLoading || busy, onLoad: loadInputSourceDir }}
            />

            <PathPickerField
              label="出力先"
              value={settings.customOutputDir ?? ""}
              placeholder="出力先フォルダ"
              onChange={(nextOutputDir) =>
                updateSettings((current) => ({
                  ...current,
                  outputMode: "custom",
                  customOutputDir: nextOutputDir,
                }))
              }
              onBrowse={pickOutputFolder}
              onReset={resetOutputPath}
            />
          </div>

          <div className="workspace-grid">
            <TablePanel
              title="入力一覧"
              count={entries.length}
              empty={entries.length === 0}
              loading={inputLoading}
              actions={
                <div className="subpanel-actions">
                  <button type="button" className="ghost panel-action" disabled={inputLoading || busy} onClick={pickFiles}>
                    ファイル追加
                  </button>
                  <button type="button" className="ghost panel-action" disabled={inputLoading || busy} onClick={pickFolder}>
                    フォルダ追加
                  </button>
                  <button
                    type="button"
                    className="ghost panel-action"
                    disabled={(entries.length === 0 && skipped.length === 0) || inputLoading || busy}
                    onClick={clearInputs}
                  >
                    入力をクリア
                  </button>
                </div>
              }
            >
              {inputLoading ? <InlineLoading message="入力ファイルを読込中..." /> : null}
              <SkippedList items={skipped} />
              <TableScroll empty={entries.length === 0}>
                <table className="data-table">
                  <thead>
                    <tr>
                      <th className="cell-path">ファイル</th>
                      <th>形式</th>
                      <th>寸法</th>
                      <th>サイズ</th>
                      <th>状態</th>
                    </tr>
                  </thead>
                  <tbody>
                    {entries.length === 0 ? (
                      <tr>
                        <td colSpan={5} className="empty-cell">
                          まだファイルがありません
                        </td>
                      </tr>
                    ) : (
                      entries.map((entry) => {
                        const preflights = computePreflight(entry, settings);
                        const hasDanger = preflights.some((item) => item.level === "danger");
                        return (
                        <tr key={entry.id}>
                          <td className="cell-path">
                            <div className="file-cell">
                              <strong
                                title={entry.fileName}
                                className={`file-name file-name-${inputNameState(entry, preflights)}`}
                              >
                                {entry.fileName}
                                {hasDanger ? (
                                  <span className="file-name-indicator">失敗予測</span>
                                ) : !entry.runtimeSupported ? (
                                  <span className="file-name-indicator">制約</span>
                                ) : preflights.length > 0 ? (
                                  <span className="file-name-indicator">注意</span>
                                ) : entry.animated ? (
                                  <span className="file-name-indicator">animation</span>
                                ) : null}
                              </strong>
                              <small title={entry.sourcePath}>{entry.sourcePath}</small>
                            </div>
                          </td>
                          <td>{entry.formatLabel}</td>
                          <td>{formatDimension(entry.width, entry.height)}</td>
                          <td>{formatBytes(entry.fileSize)}</td>
                          <td>
                            <div className="tag-list">
                              {entry.animated ? <span className="tag accent">animation</span> : null}
                              {!entry.runtimeSupported ? <span className="tag warning">runtime 制約</span> : null}
                              {preflights.map((item) => (
                                <span key={item.label} className={`tag ${item.level}`} title={item.detail}>
                                  {item.label}
                                </span>
                              ))}
                              {entry.warnings.map((warning) => (
                                <span key={warning} className="tag subtle" title={warning}>
                                  {warning}
                                </span>
                              ))}
                            </div>
                          </td>
                        </tr>
                        );
                      })
                    )}
                  </tbody>
                </table>
              </TableScroll>
            </TablePanel>

            <TablePanel
              title="結果"
              count={results.length}
              empty={results.length === 0}
              summary={
                <>
                  <span className={`saved-inline${totalSaved < 0 ? " size-increased" : ""}`}>
                    {totalSaved < 0 ? `増加: ${formatBytes(-totalSaved)}` : `Saved: ${formatBytes(totalSaved)}`}
                    {totalSavedPercent != null && totalSavedPercent !== 0
                      ? ` / ${totalSavedPercent > 0 ? "-" : "+"}${Math.abs(totalSavedPercent).toFixed(1)}%`
                      : ""}
                  </span>
                  {failedCount > 0 ? <span className="summary-pill danger">失敗: {failedCount} 件</span> : null}
                </>
              }
              actions={
                <button
                  type="button"
                  className="ghost panel-action"
                  disabled={results.length === 0 || busy}
                  onClick={clearResults}
                >
                  結果をクリア
                </button>
              }
            >
              <TableScroll empty={results.length === 0}>
                <table className="data-table">
                  <thead>
                    <tr>
                      <th className="cell-path">入力</th>
                      <th className="cell-path">出力</th>
                      <th>Original</th>
                      <th>Optimized</th>
                      <th>Saved</th>
                      <th>状態</th>
                    </tr>
                  </thead>
                  <tbody>
                    {results.length === 0 ? (
                      <tr>
                        <td colSpan={6} className="empty-cell">
                          まだ処理結果がありません
                        </td>
                      </tr>
                    ) : (
                      results.map((result) => (
                        <tr key={`${result.sourcePath}-${result.outputPath ?? "error"}`}>
                          <td className="cell-path">
                            <div className="file-cell">
                              <strong
                                title={result.sourcePath.split(/[\\/]/).pop() ?? ""}
                                className={`file-name file-name-${resultNameState(result, entryBySourcePath.get(result.sourcePath))}`}
                              >
                                {result.sourcePath.split(/[\\/]/).pop()}
                                {!result.success ? (
                                  <span className="file-name-indicator">失敗</span>
                                ) : entryBySourcePath.get(result.sourcePath)?.animated ? (
                                  <span className="file-name-indicator">animation</span>
                                ) : result.warnings.length > 0 ? (
                                  <span className="file-name-indicator">注意</span>
                                ) : null}
                              </strong>
                              <small title={result.sourcePath}>{result.sourcePath}</small>
                            </div>
                          </td>
                          <td className="cell-path">
                            <div className="file-cell">
                              <strong title={result.outputFormat ?? "-"}>{result.outputFormat ?? "-"}</strong>
                              <small title={result.outputPath ?? result.reason ?? "-"}>{result.outputPath ?? result.reason ?? "-"}</small>
                            </div>
                          </td>
                          <td>{formatBytes(result.originalSize)}</td>
                          <td>{formatBytes(result.optimizedSize)}</td>
                          <td className={result.success && (result.savedSize ?? 0) < 0 ? "size-increased" : undefined}>
                            {result.success ? formatSavedDelta(result.savedSize, result.savedPercent) : "-"}
                          </td>
                          <td>
                            <div className="tag-list">
                              <span className={`tag ${result.success ? "success" : "danger"}`}>
                                {result.success ? "success" : "failed"}
                              </span>
                              {result.warnings.map((warning) => (
                                <span key={warning} className="tag subtle" title={warning}>
                                  {warning}
                                </span>
                              ))}
                            </div>
                          </td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </TableScroll>
            </TablePanel>
          </div>

          {errorMessage ? <div className="notice danger">{errorMessage}</div> : null}
        </section>
      </section>
    </main>
  );
}
