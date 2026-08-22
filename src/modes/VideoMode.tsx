import { useEffect, useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import appIconUrl from "../assets/storageslim-icon.svg";
import { AppHeader } from "../components/AppHeader";
import { PathPickerField } from "../components/PathPickerField";
import { ProgressPanel } from "../components/ProgressPanel";
import { InlineLoading, SkippedList, TablePanel, TableScroll } from "../components/TablePanel";
import { VideoSettingsPanel } from "../components/VideoSettingsPanel";
import { useDropTarget } from "../hooks/useDropTarget";
import { formatBytes, formatDimension, formatDuration, formatSavedDelta } from "../lib/format";
import { deriveDefaultInputDir, fileNameFromPath } from "../lib/paths";
import { INITIAL_PROGRESS } from "../lib/progress";
import { isResizeValueMissing } from "../lib/settings";
import type {
  AppMode,
  BatchProgress,
  SkippedItem,
  VideoEnvironment,
  VideoInputEntry,
  VideoInspectResponse,
  VideoProcessResponse,
  VideoResultItem,
  VideoSettings,
} from "../types";

const STORAGE_KEY = "storageslim.video.settings.v1";
const INPUT_SOURCE_KEY = "storageslim.video.inputSourceDir.v1";

const fileFilters = [
  {
    name: "Videos",
    extensions: ["mp4", "mov", "m4v", "webm", "mkv", "avi", "wmv", "mts", "m2ts"],
  },
];

const RESIZE_REQUIRED_MESSAGE = "リサイズ基準を選択した場合はリサイズ値の入力が必要です。";

function createDefaultSettings(defaultOutputDir: string): VideoSettings {
  return {
    outputFormat: "mp4H264",
    outputMode: "custom",
    customOutputDir: defaultOutputDir,
    overwrite: false,
    resize: { mode: "none", value: null, unit: "px" },
    qualityPreset: "standard",
    crfOverride: null,
    fpsLimit: null,
    audioMode: "copy",
    audioBitrateKbps: 128,
    metadataMode: "strip",
    timestamps: { preserveCreationTime: false, preserveLastWriteTime: false },
    ffmpegPath: null,
  };
}

/**
 * 保存済み設定の取り込み。
 *
 * 想定外の値が入っていても既定値へ落として起動できるようにする。
 * 画像側の normalizeSettings と同じ方針。
 */
function normalizeSettings(raw: unknown, defaultOutputDir: string): VideoSettings {
  const fallback = createDefaultSettings(defaultOutputDir);
  if (!raw || typeof raw !== "object") {
    return fallback;
  }
  const candidate = raw as Partial<VideoSettings>;
  const resize = candidate.resize ?? fallback.resize;
  const timestamps = candidate.timestamps ?? fallback.timestamps;

  const qualityPreset =
    candidate.qualityPreset === "high" ||
    candidate.qualityPreset === "standard" ||
    candidate.qualityPreset === "small" ||
    candidate.qualityPreset === "smallest"
      ? candidate.qualityPreset
      : fallback.qualityPreset;

  const audioMode =
    candidate.audioMode === "copy" || candidate.audioMode === "aac" || candidate.audioMode === "remove"
      ? candidate.audioMode
      : fallback.audioMode;

  return {
    outputFormat: "mp4H264",
    outputMode: "custom",
    customOutputDir:
      typeof candidate.customOutputDir === "string" && candidate.customOutputDir.trim().length > 0
        ? candidate.customOutputDir
        : defaultOutputDir,
    overwrite: Boolean(candidate.overwrite),
    resize: {
      mode:
        resize.mode === "width" || resize.mode === "height" || resize.mode === "longEdge"
          ? resize.mode
          : "none",
      value:
        typeof resize.value === "number" && Number.isFinite(resize.value) && resize.value > 0
          ? Math.round(resize.value)
          : null,
      unit: resize.unit === "percent" ? "percent" : "px",
    },
    qualityPreset,
    crfOverride:
      typeof candidate.crfOverride === "number" && Number.isFinite(candidate.crfOverride)
        ? Math.min(51, Math.max(0, Math.round(candidate.crfOverride)))
        : null,
    fpsLimit:
      candidate.fpsLimit === 24 || candidate.fpsLimit === 30 || candidate.fpsLimit === 60
        ? candidate.fpsLimit
        : null,
    audioMode,
    audioBitrateKbps:
      candidate.audioBitrateKbps === 96 ||
      candidate.audioBitrateKbps === 128 ||
      candidate.audioBitrateKbps === 192
        ? candidate.audioBitrateKbps
        : fallback.audioBitrateKbps,
    metadataMode: candidate.metadataMode === "keep" ? "keep" : "strip",
    timestamps: {
      preserveCreationTime: Boolean(timestamps.preserveCreationTime),
      preserveLastWriteTime: Boolean(timestamps.preserveLastWriteTime),
    },
    ffmpegPath:
      typeof candidate.ffmpegPath === "string" && candidate.ffmpegPath.trim().length > 0
        ? candidate.ffmpegPath
        : null,
  };
}

function loadStoredSettings(defaultOutputDir: string): VideoSettings {
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

function mergeEntries(current: VideoInputEntry[], incoming: VideoInputEntry[]): VideoInputEntry[] {
  const merged = new Map<string, VideoInputEntry>();
  for (const entry of current) {
    merged.set(entry.sourcePath, entry);
  }
  for (const entry of incoming) {
    merged.set(entry.sourcePath, entry);
  }
  return Array.from(merged.values()).sort((a, b) => a.sourcePath.localeCompare(b.sourcePath));
}

function resultNameState(result: VideoResultItem): "normal" | "accent" | "warning" | "danger" {
  if (result.interrupted) {
    return "warning";
  }
  if (!result.success) {
    return "danger";
  }
  return result.warnings.length > 0 ? "warning" : "normal";
}

/**
 * 動画圧縮モードの画面。
 *
 * 入力一覧と結果は App が持つ（`docs/decision-log.md` の `D-17`）。
 * 停止の扱いが画像モードと異なるため、文言もここで動画向けにしている（`D-20`）。
 */
export function VideoMode({
  mode,
  onModeChange,
  entries,
  setEntries,
  skipped,
  setSkipped,
  excludedCount,
  setExcludedCount,
  results,
  setResults,
  progress,
  setProgress,
}: {
  mode: AppMode;
  onModeChange: (next: AppMode) => void;
  entries: VideoInputEntry[];
  setEntries: Dispatch<SetStateAction<VideoInputEntry[]>>;
  skipped: SkippedItem[];
  setSkipped: Dispatch<SetStateAction<SkippedItem[]>>;
  excludedCount: number;
  setExcludedCount: Dispatch<SetStateAction<number>>;
  results: VideoResultItem[];
  setResults: Dispatch<SetStateAction<VideoResultItem[]>>;
  progress: BatchProgress;
  setProgress: Dispatch<SetStateAction<BatchProgress>>;
}) {
  const [defaultOutputDir, setDefaultOutputDir] = useState("");
  const [defaultInputDir, setDefaultInputDir] = useState("");
  const [inputSourceDir, setInputSourceDir] = useState("");
  const [settings, setSettings] = useState<VideoSettings | null>(null);
  const [environment, setEnvironment] = useState<VideoEnvironment | null>(null);
  const [busy, setBusy] = useState(false);
  const [paused, setPaused] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [inputLoading, setInputLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
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
          if (!active) {
            return;
          }
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
  }, [setProgress]);

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

  // FFmpeg の所在はパス設定を変えるたびに再確認する。
  const ffmpegPath = settings?.ffmpegPath ?? null;
  useEffect(() => {
    if (!settings) {
      return;
    }
    let active = true;
    invoke<VideoEnvironment>("video_environment", { ffmpegPath })
      .then((value) => {
        if (active) {
          setEnvironment(value);
        }
      })
      .catch((error) => {
        if (active) {
          setEnvironment({
            available: false,
            ffmpegPath: null,
            ffprobePath: null,
            version: null,
            source: null,
            videoEncoder: null,
            rateControl: null,
            message: error instanceof Error ? error.message : String(error),
          });
        }
      });
    return () => {
      active = false;
    };
    // settings 全体ではなくパスだけに反応させる。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ffmpegPath, settings != null]);

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

  const totalSaved = useMemo(
    () => results.filter((result) => result.success).reduce((sum, result) => sum + (result.savedSize ?? 0), 0),
    [results],
  );
  const totalSavedPercent = useMemo(() => {
    const totalOriginal = results
      .filter((result) => result.success)
      .reduce((sum, result) => sum + (result.originalSize ?? 0), 0);
    if (totalOriginal <= 0) {
      return null;
    }
    return (totalSaved / totalOriginal) * 100;
  }, [results, totalSaved]);
  const failedCount = useMemo(
    () => results.filter((result) => !result.success && !result.interrupted).length,
    [results],
  );
  const interruptedCount = useMemo(() => results.filter((result) => result.interrupted).length, [results]);

  const resizeValueMissing = settings ? isResizeValueMissing(settings.resize) : true;
  const ffmpegReady = environment?.available === true;
  const canRunBatch =
    entries.length > 0 && !busy && !inputLoading && !resizeValueMissing && ffmpegReady;

  async function addPaths(paths: string[]) {
    if (paths.length === 0) {
      return;
    }
    try {
      const response = await invoke<VideoInspectResponse>("inspect_video_inputs", {
        paths,
        ffmpegPath,
      });
      setEntries((current) => mergeEntries(current, response.entries));
      setSkipped((current) => current.concat(response.skipped));
      setExcludedCount((current) => current + response.excludedCount);
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
      // フォルダの再帰探索は Rust 側の inspect が行うので、そのまま渡す。
      const response = await invoke<VideoInspectResponse>("inspect_video_inputs", {
        paths: [path],
        ffmpegPath,
      });
      if (response.entries.length === 0 && response.skipped.length === 0) {
        setErrorMessage("入力先フォルダ内に対応動画が見つかりませんでした。");
        return;
      }
      setEntries((current) => mergeEntries(current, response.entries));
      setSkipped((current) => current.concat(response.skipped));
      setExcludedCount((current) => current + response.excludedCount);
      setErrorMessage(null);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setInputLoading(false);
    }
  }

  async function pickFiles() {
    const selected = await open({ multiple: true, filters: fileFilters });
    if (!selected) {
      return;
    }
    await addPaths(Array.isArray(selected) ? selected : [selected]);
  }

  async function pickFolder() {
    const selected = await open({ directory: true, multiple: false });
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
    updateSettings((current) => ({ ...current, outputMode: "custom", customOutputDir: selected }));
  }

  async function runBatch() {
    if (!settings || entries.length === 0 || busy) {
      return;
    }
    if (resizeValueMissing) {
      setErrorMessage(RESIZE_REQUIRED_MESSAGE);
      return;
    }
    if (!ffmpegReady) {
      setErrorMessage(environment?.message ?? "FFmpeg が利用できません。");
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
      currentFilePercent: 0,
    });
    try {
      const response = await invoke<VideoProcessResponse>("process_video_batch", {
        request: { entries, settings },
      });
      setResults(response.results);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
      setPaused(false);
      setStopping(false);
      setProgress((current) => ({ ...current, currentPath: null, currentFilePercent: null }));
    }
  }

  function clearInputs() {
    setEntries([]);
    setSkipped([]);
    setExcludedCount(0);
  }

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

  function updateSettings(updater: (current: VideoSettings) => VideoSettings) {
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
      ? "停止中: 現在のファイルの出力を破棄します"
      : pausedEffective
        ? "一時停止中: 次のファイルの前で止まります"
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
        tagline="動画最適化ワークベンチ"
        version={appVersion}
        mode={mode}
        onModeChange={onModeChange}
        switchDisabled={busy || inputLoading}
      />

      <section className="app-grid">
        <VideoSettingsPanel
          settings={settings}
          updateSettings={updateSettings}
          onResetAll={resetAllSettings}
          clearResizeError={() =>
            setErrorMessage((current) => (current === RESIZE_REQUIRED_MESSAGE ? null : current))
          }
          environment={environment}
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
                filePercent={busy ? progress.currentFilePercent : null}
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
                      {pausedEffective ? "再開" : "次のファイルの前で一時停止"}
                    </button>
                    <button
                      type="button"
                      className="danger run-control"
                      disabled={stoppingEffective}
                      onClick={stopBatch}
                      title="現在のファイルの出力を破棄して停止します"
                    >
                      {stoppingEffective ? "停止中..." : "停止"}
                    </button>
                  </>
                ) : (
                  <button type="button" className="primary run-button" disabled={!canRunBatch} onClick={runBatch}>
                    圧縮を実行
                  </button>
                )}
              </div>
            </div>
          </div>

          {!ffmpegReady ? (
            <div className="notice danger">
              {environment?.message ?? "FFmpeg を確認しています..."}
            </div>
          ) : null}

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
                    disabled={(entries.length === 0 && skipped.length === 0 && excludedCount === 0) || inputLoading || busy}
                    onClick={clearInputs}
                  >
                    入力をクリア
                  </button>
                </div>
              }
            >
              {inputLoading ? <InlineLoading message="入力ファイルを読込中..." /> : null}
              {excludedCount > 0 ? (
                <div className="notice">
                  対象外の種別 {excludedCount} 件は読み込んでいません。画像は「画像圧縮」モードで処理してください。
                </div>
              ) : null}
              <SkippedList items={skipped} />
              <TableScroll empty={entries.length === 0}>
                <table className="data-table">
                  <thead>
                    <tr>
                      <th className="cell-path">ファイル</th>
                      <th>形式</th>
                      <th>寸法</th>
                      <th>尺</th>
                      <th>サイズ</th>
                      <th>状態</th>
                    </tr>
                  </thead>
                  <tbody>
                    {entries.length === 0 ? (
                      <tr>
                        <td colSpan={6} className="empty-cell">
                          まだファイルがありません
                        </td>
                      </tr>
                    ) : (
                      entries.map((entry) => (
                        <tr key={entry.id}>
                          <td className="cell-path">
                            <div className="file-cell">
                              <strong
                                title={entry.fileName}
                                className={`file-name file-name-${entry.warnings.length > 0 ? "warning" : "normal"}`}
                              >
                                {entry.fileName}
                                {entry.warnings.length > 0 ? (
                                  <span className="file-name-indicator">注意</span>
                                ) : null}
                              </strong>
                              <small title={entry.sourcePath}>{entry.sourcePath}</small>
                            </div>
                          </td>
                          <td>{entry.formatLabel}</td>
                          <td>{formatDimension(entry.width, entry.height)}</td>
                          <td>{formatDuration(entry.durationSec)}</td>
                          <td>{formatBytes(entry.fileSize)}</td>
                          <td>
                            <div className="tag-list">
                              {entry.fps != null ? (
                                <span className="tag subtle">{entry.fps.toFixed(entry.fps % 1 === 0 ? 0 : 2)} fps</span>
                              ) : null}
                              {entry.hasAudio ? (
                                <span className="tag subtle">{entry.audioCodec ?? "audio"}</span>
                              ) : (
                                <span className="tag subtle">音声なし</span>
                              )}
                              {entry.warnings.map((warning) => (
                                <span key={warning} className="tag warning" title={warning}>
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
                  {interruptedCount > 0 ? (
                    <span className="summary-pill">中断: {interruptedCount} 件</span>
                  ) : null}
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
                                className={`file-name file-name-${resultNameState(result)}`}
                              >
                                {result.sourcePath.split(/[\\/]/).pop()}
                                {result.interrupted ? (
                                  <span className="file-name-indicator">中断</span>
                                ) : !result.success ? (
                                  <span className="file-name-indicator">失敗</span>
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
                              <small title={result.outputPath ?? result.reason ?? "-"}>
                                {result.outputPath ?? result.reason ?? "-"}
                              </small>
                            </div>
                          </td>
                          <td>{formatBytes(result.originalSize)}</td>
                          <td>{formatBytes(result.optimizedSize)}</td>
                          <td className={result.success && (result.savedSize ?? 0) < 0 ? "size-increased" : undefined}>
                            {result.success ? formatSavedDelta(result.savedSize, result.savedPercent) : "-"}
                          </td>
                          <td>
                            <div className="tag-list">
                              <span
                                className={`tag ${
                                  result.interrupted ? "warning" : result.success ? "success" : "danger"
                                }`}
                              >
                                {result.interrupted ? "中断" : result.success ? "success" : "failed"}
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
