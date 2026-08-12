import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { readDir } from "@tauri-apps/plugin-fs";
import "./App.css";
import type {
  BatchProgress,
  BatchSettings,
  InputEntry,
  InspectResponse,
  OutputFormat,
  ProcessResponse,
  ResizeMode,
  ResizeUnit,
  SkippedItem,
} from "./types";

type ChoiceOption<T extends string> = {
  value: T;
  label: string;
  disabled?: boolean;
  title?: string;
};

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

function joinNativePath(parent: string, child: string): string {
  return `${parent.replace(/[\\/]+$/, "")}\\${child}`;
}

const outputOptions: Array<{ value: OutputFormat; label: string }> = [
  { value: "original", label: "オリジナル維持" },
  { value: "gif", label: "GIF" },
  { value: "jpeg", label: "JPEG" },
  { value: "png", label: "PNG" },
  { value: "webp", label: "WebP" },
  { value: "avif", label: "AVIF" },
];

const resizeModeOptions: Array<ChoiceOption<ResizeMode>> = [
  { value: "none", label: "変更なし" },
  { value: "width", label: "幅" },
  { value: "height", label: "高さ" },
  { value: "longEdge", label: "長辺" },
];

const resizeUnitOptions: Array<ChoiceOption<ResizeUnit>> = [
  { value: "px", label: "px" },
  { value: "percent", label: "%" },
];

const metadataOptions: Array<ChoiceOption<BatchSettings["metadataMode"]>> = [
  { value: "strip", label: "削除する" },
  { value: "keep", label: "保持する" },
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
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function roundToStep(value: number, min: number, max: number, step: number): number {
  const snapped = Math.round((value - min) / step) * step + min;
  return clamp(Number(snapped.toFixed(4)), min, max);
}

function deriveDefaultInputDir(defaultOutputDir: string): string {
  if (!defaultOutputDir) {
    return "Desktop/@StorageSlim/input";
  }
  if (/[\\/]output$/i.test(defaultOutputDir)) {
    return defaultOutputDir.replace(/[\\/]output$/i, (match) => match.replace(/output/i, "input"));
  }
  return `${defaultOutputDir.replace(/[\\/]?$/, "")}\\input`;
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
      candidate.metadataMode === "keep" || candidate.metadataMode === "strip"
        ? candidate.metadataMode
        : fallback.metadataMode,
    timestamps: {
      preserveCreationTime: Boolean(timestamps.preserveCreationTime),
      preserveLastWriteTime: Boolean(timestamps.preserveLastWriteTime),
    },
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

function formatBytes(bytes: number | null): string {
  if (bytes == null) {
    return "-";
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(value >= 100 ? 0 : 1)} ${units[unitIndex]}`;
}

function formatDimension(width: number | null, height: number | null): string {
  if (!width || !height) {
    return "-";
  }
  return `${width} x ${height}`;
}

function fileNameFromPath(path: string | null): string | null {
  if (!path) {
    return null;
  }
  return path.split(/[\\/]/).pop() ?? path;
}

function inputNameState(entry: InputEntry): "normal" | "accent" | "warning" {
  if (!entry.runtimeSupported) {
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

function ChoiceGroup<T extends string>({
  value,
  options,
  onChange,
  disabled = false,
}: {
  value: T;
  options: Array<ChoiceOption<T>>;
  onChange: (nextValue: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className={`choice-group ${disabled ? "is-disabled" : ""}`}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={`choice-chip ${value === option.value ? "active" : ""}`}
          aria-pressed={value === option.value}
          disabled={disabled || option.disabled}
          title={option.title}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function CustomSlider({
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (nextValue: number) => void;
}) {
  const trackRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);

  useEffect(() => {
    function updateFromClientX(clientX: number) {
      const track = trackRef.current;
      if (!track) {
        return;
      }
      const rect = track.getBoundingClientRect();
      const ratio = clamp((clientX - rect.left) / rect.width, 0, 1);
      const nextValue = roundToStep(min + ratio * (max - min), min, max, step);
      onChange(nextValue);
    }

    function handleMove(event: MouseEvent) {
      if (!draggingRef.current) {
        return;
      }
      updateFromClientX(event.clientX);
    }

    function handleUp() {
      draggingRef.current = false;
    }

    window.addEventListener("mousemove", handleMove);
    window.addEventListener("mouseup", handleUp);
    return () => {
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
    };
  }, [max, min, onChange, step]);

  const ratio = ((value - min) / (max - min)) * 100;

  return (
    <div
      ref={trackRef}
      className="custom-slider"
      onMouseDown={(event) => {
        draggingRef.current = true;
        const rect = trackRef.current?.getBoundingClientRect();
        if (!rect) {
          return;
        }
        const nextValue = roundToStep(min + ((event.clientX - rect.left) / rect.width) * (max - min), min, max, step);
        onChange(nextValue);
      }}
    >
      <div className="custom-slider-fill" style={{ width: `${ratio}%` }} />
      <div className="custom-slider-thumb" style={{ left: `${ratio}%` }} />
    </div>
  );
}

function QualityField({
  label,
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (nextValue: number) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(String(value));

  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  function commit(nextRaw: string) {
    const parsed = Number(nextRaw);
    const next = roundToStep(Number.isFinite(parsed) ? parsed : value, min, max, step);
    onChange(next);
    setDraft(String(next));
    setEditing(false);
  }

  return (
    <div className="value-card">
      <div className="value-card-header">
        <span>{label}</span>
        {editing ? (
          <input
            className="value-inline-input"
            type="number"
            min={min}
            max={max}
            step={step}
            value={draft}
            autoFocus
            onChange={(event) => {
              const nextRaw = event.currentTarget.value;
              setDraft(nextRaw);
              if (nextRaw === "") {
                return;
              }
              const parsed = Number(nextRaw);
              if (Number.isFinite(parsed)) {
                onChange(roundToStep(parsed, min, max, step));
              }
            }}
            onBlur={() => commit(draft)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                commit(draft);
              }
              if (event.key === "Escape") {
                setDraft(String(value));
                setEditing(false);
              }
            }}
          />
        ) : (
          <button type="button" className="value-chip" onClick={() => setEditing(true)}>
            {value}
          </button>
        )}
      </div>
      <CustomSlider value={value} min={min} max={max} step={step} onChange={onChange} />
    </div>
  );
}

function App() {
  const [defaultOutputDir, setDefaultOutputDir] = useState("");
  const [defaultInputDir, setDefaultInputDir] = useState("");
  const [inputSourceDir, setInputSourceDir] = useState("");
  const [settings, setSettings] = useState<BatchSettings | null>(null);
  const [entries, setEntries] = useState<InputEntry[]>([]);
  const [skipped, setSkipped] = useState<SkippedItem[]>([]);
  const [results, setResults] = useState<ProcessResponse["results"]>([]);
  const [progress, setProgress] = useState<BatchProgress>({
    completed: 0,
    total: 0,
    currentPath: null,
  });
  const [busy, setBusy] = useState(false);
  const [paused, setPaused] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [inputLoading, setInputLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [dropActive, setDropActive] = useState(false);
  const [advancedExpanded, setAdvancedExpanded] = useState(false);
  const dropDepthRef = useRef(0);

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
    let unlistenDrop: (() => void) | undefined;
    let unlistenProgress: (() => void) | undefined;

    async function bind() {
      try {
        const webview = getCurrentWebview();
        unlistenDrop = await webview.onDragDropEvent(async (event) => {
          if (event.payload.type === "enter" || event.payload.type === "over") {
            if (active) {
              setDropActive(true);
            }
            return;
          }
          if (event.payload.type === "leave") {
            if (active) {
              setDropActive(false);
            }
            return;
          }
          if (event.payload.type === "drop") {
            if (active) {
              setDropActive(false);
            }
            await addPaths(event.payload.paths);
          }
        });
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
        console.warn("StorageSlim: Tauri webview binding is unavailable in this environment.", error);
      }
    }

    void bind();

    const handleDragEnter = (event: DragEvent) => {
      event.preventDefault();
      dropDepthRef.current += 1;
      setDropActive(true);
    };

    const handleDragOver = (event: DragEvent) => {
      event.preventDefault();
      setDropActive(true);
    };

    const handleDragLeave = (event: DragEvent) => {
      event.preventDefault();
      dropDepthRef.current = Math.max(0, dropDepthRef.current - 1);
      if (dropDepthRef.current === 0) {
        setDropActive(false);
      }
    };

    const handleDrop = (event: DragEvent) => {
      event.preventDefault();
      dropDepthRef.current = 0;
      setDropActive(false);
    };

    window.addEventListener("dragenter", handleDragEnter);
    window.addEventListener("dragover", handleDragOver);
    window.addEventListener("dragleave", handleDragLeave);
    window.addEventListener("drop", handleDrop);

    return () => {
      active = false;
      unlistenDrop?.();
      unlistenProgress?.();
      window.removeEventListener("dragenter", handleDragEnter);
      window.removeEventListener("dragover", handleDragOver);
      window.removeEventListener("dragleave", handleDragLeave);
      window.removeEventListener("drop", handleDrop);
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

  const resizeValueDisabled = settings?.resize.mode === "none";
  const resizeValueUnit = settings?.resize.unit ?? "px";
  const resizeValueMax = resizeValueUnit === "percent" ? 100 : 100000;
  const resizeValueRequired = !resizeValueDisabled;
  const resizeValueMissing = resizeValueRequired && (settings?.resize.value == null || settings.resize.value <= 0);
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
          <p className="eyebrow">StorageSlim MVP</p>
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
      <section className="app-header panel">
        <div>
          <p className="eyebrow">StorageSlim MVP</p>
          <h1>画像最適化ワークベンチ</h1>
        </div>
      </section>

      <section className="app-grid">
        <aside className="panel settings-panel">
          <div className="panel-header">
            <div className="title-inline">
              <h2>設定</h2>
              <button type="button" className="ghost micro-button" onClick={resetAllSettings}>
                初期化
              </button>
            </div>
          </div>

          <div className="settings-stack">
            <div className="field setting-output-format">
              <span>出力形式</span>
              <ChoiceGroup
                value={settings.outputFormat}
                options={outputFormatChoices}
                onChange={(outputFormat) => updateSettings((current) => ({ ...current, outputFormat }))}
              />
            </div>

            <div className="field setting-resize-mode">
              <span>リサイズ基準</span>
              <div className={`resize-control-row ${resizeValueMissing ? "is-required" : ""}`}>
                <ChoiceGroup
                  value={settings.resize.mode}
                  options={resizeModeOptions}
                  onChange={(mode) =>
                    updateSettings((current) => ({
                      ...current,
                      resize: {
                        ...current.resize,
                        mode,
                      },
                    }))
                  }
                />
                <div className="resize-value-inline">
                  <input
                    type="number"
                    min={1}
                    max={resizeValueMax}
                    disabled={resizeValueDisabled}
                    aria-invalid={resizeValueMissing}
                    value={settings.resize.value ?? ""}
                    placeholder={resizeValueMissing ? "必須" : ""}
                    onChange={(event) => {
                      const rawValue = event.currentTarget.value;
                      if (rawValue !== "") {
                        setErrorMessage((current) =>
                          current === "リサイズ基準を選択した場合はリサイズ値の入力が必要です。" ? null : current,
                        );
                      }
                      updateSettings((current) => {
                        const parsed = Number(rawValue);
                        return {
                          ...current,
                          resize: {
                            ...current.resize,
                            value:
                              rawValue === ""
                                ? null
                                : Number.isFinite(parsed)
                                  ? clamp(Math.round(parsed), 1, resizeValueMax)
                                  : current.resize.value,
                          },
                        };
                      });
                    }}
                  />
                  <ChoiceGroup
                    value={settings.resize.unit}
                    options={resizeUnitOptions}
                    disabled={resizeValueDisabled}
                    onChange={(unit) =>
                      updateSettings((current) => ({
                        ...current,
                        resize: {
                          ...current.resize,
                          unit,
                          value:
                            current.resize.value == null
                              ? null
                              : clamp(Math.round(current.resize.value), 1, unit === "percent" ? 100 : 100000),
                        },
                      }))
                    }
                  />
                </div>
              </div>
            </div>

            <div className="field setting-metadata">
              <span>メタデータ</span>
              <ChoiceGroup
                value={settings.metadataMode}
                options={metadataOptions}
                onChange={(metadataMode) => updateSettings((current) => ({ ...current, metadataMode }))}
              />
            </div>

            <div className="advanced-toggle">
              <button
                type="button"
                className={`section-disclosure ${advancedExpanded ? "is-open" : ""}`}
                aria-expanded={advancedExpanded}
                onClick={() => setAdvancedExpanded((current) => !current)}
              >
                <span className="section-disclosure-copy">
                  <strong>品質調整・その他</strong>
                </span>
                <span className="section-disclosure-chevron" aria-hidden="true">
                  {advancedExpanded ? "▾" : "▸"}
                </span>
              </button>
            </div>

            {advancedExpanded ? (
              <div className="quality-grid">
                <QualityField
                  label="JPEG (1-100)"
                  value={settings.quality.jpegQuality}
                  min={1}
                  max={100}
                  onChange={(jpegQuality) =>
                    updateSettings((current) => ({
                      ...current,
                      quality: { ...current.quality, jpegQuality },
                    }))
                  }
                />
                <QualityField
                  label="WebP (1-100)"
                  value={settings.quality.webpQuality}
                  min={1}
                  max={100}
                  onChange={(webpQuality) =>
                    updateSettings((current) => ({
                      ...current,
                      quality: { ...current.quality, webpQuality },
                    }))
                  }
                />
                <QualityField
                  label="AVIF (1-100)"
                  value={settings.quality.avifQuality}
                  min={1}
                  max={100}
                  onChange={(avifQuality) =>
                    updateSettings((current) => ({
                      ...current,
                      quality: { ...current.quality, avifQuality },
                    }))
                  }
                />
                <QualityField
                  label="PNG (0-9)"
                  value={settings.quality.pngCompression}
                  min={0}
                  max={9}
                  onChange={(pngCompression) =>
                    updateSettings((current) => ({
                      ...current,
                      quality: { ...current.quality, pngCompression },
                    }))
                  }
                />
                <QualityField
                  label="GIF (2-256)"
                  value={settings.quality.gifColors}
                  min={2}
                  max={256}
                  onChange={(gifColors) =>
                    updateSettings((current) => ({
                      ...current,
                      quality: { ...current.quality, gifColors },
                    }))
                  }
                />
              </div>
            ) : null}

            {advancedExpanded ? (
              <div className="field setting-other-panel">
                <div className="checkbox-cluster">
                  <label className="checkbox">
                    <input
                      type="checkbox"
                      checked={settings.timestamps.preserveCreationTime}
                      onChange={(event) => {
                        const checked = event.currentTarget.checked;
                        updateSettings((current) => ({
                          ...current,
                          timestamps: {
                            ...current.timestamps,
                            preserveCreationTime: checked,
                          },
                        }));
                      }}
                    />
                    <span>作成日時を引き継ぐ</span>
                  </label>
                  <label className="checkbox">
                    <input
                      type="checkbox"
                      checked={settings.timestamps.preserveLastWriteTime}
                      onChange={(event) => {
                        const checked = event.currentTarget.checked;
                        updateSettings((current) => ({
                          ...current,
                          timestamps: {
                            ...current.timestamps,
                            preserveLastWriteTime: checked,
                          },
                        }));
                      }}
                    />
                    <span>更新日時を引き継ぐ</span>
                  </label>
                  <label className="checkbox">
                    <input
                      type="checkbox"
                      checked={settings.overwrite}
                      onChange={(event) => {
                        const checked = event.currentTarget.checked;
                        updateSettings((current) => ({
                          ...current,
                          overwrite: checked,
                        }));
                      }}
                    />
                    <span>上書きを許可する</span>
                  </label>
                </div>
              </div>
            ) : null}
          </div>
        </aside>

        <section className={`panel workspace-panel ${dropActive ? "drop-active" : ""}`}>
          <div className="workspace-header">
            <div className="workspace-heading">
              <h2>入力と結果</h2>
              <p className="title-note">ファイル / フォルダはこの画面へドラッグ&ドロップでも追加できます</p>
            </div>
            <div className="run-panel">
              <div className="progress-panel">
                <div className="progress-inline-meta">
                  <div className="progress-summary">
                    <strong>
                      {progress.completed} / {progress.total}
                    </strong>
                    {failedCount > 0 ? <span className="summary-pill danger">失敗: {failedCount} 件</span> : null}
                  </div>
                  <span title={progress.currentPath ?? undefined}>{progressLabel}</span>
                </div>
                <div className="progress-bar">
                  <div
                    className="progress-bar-fill"
                    style={{
                      width: progress.total === 0 ? "0%" : `${Math.round((progress.completed / progress.total) * 100)}%`,
                    }}
                  />
                </div>
              </div>
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
            <div className="field">
              <div className="field-inline-head">
                <span>入力先</span>
                <button type="button" className="ghost micro-button" onClick={resetInputPath}>
                  既定値へ戻す
                </button>
              </div>
              <div className="inline-picker triple-line">
                <input
                  value={inputSourceDir}
                  placeholder="入力先フォルダ"
                  onChange={(event) => setInputSourceDir(event.currentTarget.value)}
                />
                <button type="button" className="ghost" onClick={pickInputFolder}>
                  参照
                </button>
                <button type="button" className="ghost" disabled={inputLoading || busy} onClick={loadInputSourceDir}>
                  読込
                </button>
              </div>
            </div>

            <div className="field">
              <div className="field-inline-head">
                <span>出力先</span>
                <button type="button" className="ghost micro-button" onClick={resetOutputPath}>
                  既定値へ戻す
                </button>
              </div>
              <div className="inline-picker double-line">
                <input
                  value={settings.customOutputDir ?? ""}
                  placeholder="出力先フォルダ"
                  onChange={(event) => {
                    const nextOutputDir = event.currentTarget.value;
                    updateSettings((current) => ({
                      ...current,
                      outputMode: "custom",
                      customOutputDir: nextOutputDir,
                    }));
                  }}
                />
                <button type="button" className="ghost" onClick={pickOutputFolder}>
                  参照
                </button>
              </div>
            </div>
          </div>

          <div className="workspace-grid">
            <section className={`subpanel ${entries.length === 0 ? "is-empty" : "has-rows"} ${inputLoading ? "is-loading" : ""}`}>
              <div className="subpanel-header">
                <div className="title-inline">
                  <h3>入力一覧</h3>
                  <span>{entries.length} 件</span>
                </div>
                <div className="subpanel-actions">
                  <button type="button" className="ghost panel-action" disabled={inputLoading || busy} onClick={pickFiles}>
                    ファイル追加
                  </button>
                  <button type="button" className="ghost panel-action" disabled={inputLoading || busy} onClick={pickFolder}>
                    フォルダ追加
                  </button>
                  <button type="button" className="ghost panel-action" disabled={entries.length === 0 || inputLoading || busy} onClick={() => setEntries([])}>
                    入力をクリア
                  </button>
                </div>
              </div>
              {inputLoading ? (
                <div className="inline-loading" role="status" aria-live="polite">
                  <span className="loading-dot" />
                  <span>入力ファイルを読込中...</span>
                  <div className="loading-track">
                    <div className="loading-track-fill" />
                  </div>
                </div>
              ) : null}
              {skipped.length > 0 ? (
                <details className="skip-details" open>
                  <summary>読み込めなかった項目: {skipped.length} 件</summary>
                  <div className="skip-list">
                    {skipped.map((item) => (
                      <div key={`${item.path}-${item.reason}`} className="skip-item">
                        <strong>{item.path.split(/[\\/]/).pop()}</strong>
                        <small>{item.path}</small>
                        <p>{item.reason}</p>
                      </div>
                    ))}
                  </div>
                </details>
              ) : null}
              <div className={`table-scroll ${entries.length === 0 ? "is-empty" : "has-rows"}`}>
                <table className="data-table">
                  <thead>
                    <tr>
                      <th>ファイル</th>
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
                      entries.map((entry) => (
                        <tr key={entry.id}>
                          <td>
                            <div className="file-cell">
                              <strong title={entry.fileName} className={`file-name file-name-${inputNameState(entry)}`}>
                                {entry.fileName}
                                {!entry.runtimeSupported ? (
                                  <span className="file-name-indicator">制約</span>
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
                              {entry.warnings.map((warning) => (
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
              </div>
            </section>

            <section className={`subpanel ${results.length === 0 ? "is-empty" : "has-rows"}`}>
              <div className="subpanel-header">
                <div className="title-inline">
                  <h3>結果</h3>
                  <span>{results.length} 件</span>
                  <span className="saved-inline">
                    Saved: {formatBytes(totalSaved)}
                    {totalSavedPercent != null ? ` / ${totalSavedPercent >= 0 ? "-" : "+"}${Math.abs(totalSavedPercent).toFixed(1)}%` : ""}
                  </span>
                  {failedCount > 0 ? <span className="summary-pill danger">失敗: {failedCount} 件</span> : null}
                </div>
                <button type="button" className="ghost panel-action" disabled={results.length === 0} onClick={() => setResults([])}>
                  結果をクリア
                </button>
              </div>
              <div className={`table-scroll ${results.length === 0 ? "is-empty" : "has-rows"}`}>
                <table className="data-table">
                  <thead>
                    <tr>
                      <th>入力</th>
                      <th>出力</th>
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
                          <td>
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
                          <td>
                            <div className="file-cell">
                              <strong title={result.outputFormat ?? "-"}>{result.outputFormat ?? "-"}</strong>
                              <small title={result.outputPath ?? result.reason ?? "-"}>{result.outputPath ?? result.reason ?? "-"}</small>
                            </div>
                          </td>
                          <td>{formatBytes(result.originalSize)}</td>
                          <td>{formatBytes(result.optimizedSize)}</td>
                          <td>
                            {result.success && result.savedSize != null && result.savedPercent != null
                              ? `${formatBytes(result.savedSize)} / ${result.savedPercent.toFixed(1)}%`
                              : "-"}
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
              </div>
            </section>
          </div>

          {errorMessage ? <div className="notice danger">{errorMessage}</div> : null}
        </section>
      </section>
    </main>
  );
}

export default App;
