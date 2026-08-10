import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";
import type {
  BatchProgress,
  BatchSettings,
  InputEntry,
  InspectResponse,
  OutputFormat,
  ProcessResponse,
  ResizeMode,
} from "./types";

const fileFilters = [
  {
    name: "Images",
    extensions: ["gif", "jpg", "jpeg", "png", "webp", "avif", "heic", "heif"],
  },
];

const outputOptions: Array<{ value: OutputFormat; label: string }> = [
  { value: "original", label: "オリジナル形式を維持" },
  { value: "gif", label: "GIF" },
  { value: "jpeg", label: "JPEG" },
  { value: "png", label: "PNG" },
  { value: "webp", label: "WebP" },
  { value: "avif", label: "AVIF" },
];

const initialSettings: BatchSettings = {
  outputFormat: "original",
  outputMode: "desktopDefault",
  customOutputDir: null,
  overwrite: false,
  resize: {
    mode: "none",
    value: null,
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
    preserveCreationTime: true,
    preserveLastWriteTime: true,
  },
};

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
      return "アニメーション GIF を含むため、この出力形式は使えません。";
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

function App() {
  const [entries, setEntries] = useState<InputEntry[]>([]);
  const [skipped, setSkipped] = useState<string[]>([]);
  const [settings, setSettings] = useState<BatchSettings>(initialSettings);
  const [results, setResults] = useState<ProcessResponse["results"]>([]);
  const [progress, setProgress] = useState<BatchProgress>({
    completed: 0,
    total: 0,
    currentPath: null,
  });
  const [busy, setBusy] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    let unlistenDrop: (() => void) | undefined;
    let unlistenProgress: (() => void) | undefined;

    async function bind() {
      const webview = getCurrentWebview();
      unlistenDrop = await webview.onDragDropEvent(async (event) => {
        if (event.payload.type === "drop") {
          await addPaths(event.payload.paths);
        }
      });
      unlistenProgress = await webview.listen<BatchProgress>("batch-progress", (event) => {
        if (active) {
          setProgress(event.payload);
        }
      });
    }

    void bind();

    return () => {
      active = false;
      unlistenDrop?.();
      unlistenProgress?.();
    };
  }, []);

  const allowedOutputs = useMemo(() => {
    const map = new Map<OutputFormat, string | null>();
    for (const option of outputOptions) {
      map.set(option.value, outputDisabledReason(entries, option.value));
    }
    return map;
  }, [entries]);

  useEffect(() => {
    const reason = allowedOutputs.get(settings.outputFormat);
    if (!reason) {
      return;
    }
    setSettings((current) => ({
      ...current,
      outputFormat: "original",
    }));
  }, [allowedOutputs, settings.outputFormat]);

  const summary = useMemo(() => {
    const success = results.filter((result) => result.success);
    return {
      success: success.length,
      failed: results.length - success.length,
      saved: success.reduce((sum, result) => sum + (result.savedSize ?? 0), 0),
    };
  }, [results]);

  async function addPaths(paths: string[]) {
    if (paths.length === 0) {
      return;
    }
    const response = await invoke<InspectResponse>("inspect_inputs", { paths });
    setEntries((current) => mergeEntries(current, response.entries));
    setSkipped((current) => current.concat(response.skipped));
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

  async function pickOutputFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
    });
    if (!selected || Array.isArray(selected)) {
      return;
    }
    setSettings((current) => ({
      ...current,
      outputMode: "custom",
      customOutputDir: selected,
    }));
  }

  async function runBatch() {
    if (entries.length === 0 || busy) {
      return;
    }
    setBusy(true);
    setResults([]);
    setErrorMessage(null);
    setProgress({
      completed: 0,
      total: entries.length,
      currentPath: null,
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
      setProgress((current) => ({
        ...current,
        currentPath: null,
      }));
    }
  }

  return (
    <main className="app-shell">
      <section className="hero-panel">
        <div>
          <p className="eyebrow">StorageSlim MVP</p>
          <h1>ローカル完結の画像最適化ワークベンチ</h1>
          <p className="hero-copy">
            リサイズ、品質調整、フォーマット変換、タイムスタンプ補正を 1 画面でまとめて扱います。
          </p>
        </div>
        <div className="hero-stats">
          <div>
            <span>投入ファイル</span>
            <strong>{entries.length}</strong>
          </div>
          <div>
            <span>Saved</span>
            <strong>{formatBytes(summary.saved)}</strong>
          </div>
        </div>
      </section>

      <section className="top-grid">
        <div className="panel">
          <div className="drop-header">
            <div>
              <h2>入力</h2>
              <p>ドラッグ&ドロップ、複数ファイル選択、フォルダ投入に対応します。</p>
            </div>
            <div className="drop-actions">
              <button type="button" onClick={pickFiles}>
                ファイル追加
              </button>
              <button type="button" onClick={pickFolder}>
                フォルダ追加
              </button>
              <button type="button" className="ghost" onClick={() => setEntries([])}>
                クリア
              </button>
            </div>
          </div>
          <div className="dropzone">
            <span>ここへ画像またはフォルダをドロップ</span>
            <small>対応: GIF / JPEG / PNG / WebP / AVIF / HEIC / HEIF</small>
          </div>
          {skipped.length > 0 ? (
            <div className="notice warning">対応外または読み取れなかった項目: {skipped.length} 件</div>
          ) : null}
        </div>

        <div className="panel">
          <div className="panel-header">
            <div>
              <h2>設定</h2>
              <p>出力先、品質、メタデータ、タイムスタンプをまとめて定義します。</p>
            </div>
          </div>

          <div className="field-grid">
            <label>
              <span>出力形式</span>
              <select
                value={settings.outputFormat}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    outputFormat: event.currentTarget.value as OutputFormat,
                  }))
                }
              >
                {outputOptions.map((option) => (
                  <option
                    key={option.value}
                    value={option.value}
                    disabled={Boolean(allowedOutputs.get(option.value))}
                  >
                    {option.label}
                  </option>
                ))}
              </select>
              <small>{allowedOutputs.get(settings.outputFormat) ?? "現在の入力構成で有効です。"}</small>
            </label>

            <label>
              <span>出力先</span>
              <select
                value={settings.outputMode}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    outputMode: event.currentTarget.value as BatchSettings["outputMode"],
                  }))
                }
              >
                <option value="desktopDefault">Desktop/@StorageSlim/output</option>
                <option value="custom">ユーザー指定フォルダ</option>
              </select>
            </label>

            {settings.outputMode === "custom" ? (
              <label className="full-width">
                <span>カスタム出力先</span>
                <div className="inline-picker">
                  <input value={settings.customOutputDir ?? ""} readOnly placeholder="未選択" />
                  <button type="button" onClick={pickOutputFolder}>
                    選択
                  </button>
                </div>
              </label>
            ) : null}

            <label>
              <span>リサイズ基準</span>
              <select
                value={settings.resize.mode}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    resize: {
                      ...current.resize,
                      mode: event.currentTarget.value as ResizeMode,
                    },
                  }))
                }
              >
                <option value="none">変更しない</option>
                <option value="width">幅指定</option>
                <option value="height">高さ指定</option>
                <option value="longEdge">長辺指定</option>
              </select>
            </label>

            <label>
              <span>リサイズ値</span>
              <input
                type="number"
                min={1}
                disabled={settings.resize.mode === "none"}
                value={settings.resize.value ?? ""}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    resize: {
                      ...current.resize,
                      value: event.currentTarget.value === "" ? null : Number(event.currentTarget.value),
                    },
                  }))
                }
              />
            </label>

            <label>
              <span>JPEG 品質</span>
              <input
                type="range"
                min={1}
                max={100}
                value={settings.quality.jpegQuality}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    quality: {
                      ...current.quality,
                      jpegQuality: Number(event.currentTarget.value),
                    },
                  }))
                }
              />
              <small>{settings.quality.jpegQuality}</small>
            </label>

            <label>
              <span>WebP 品質</span>
              <input
                type="range"
                min={1}
                max={100}
                value={settings.quality.webpQuality}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    quality: {
                      ...current.quality,
                      webpQuality: Number(event.currentTarget.value),
                    },
                  }))
                }
              />
              <small>{settings.quality.webpQuality}</small>
            </label>

            <label>
              <span>AVIF 品質</span>
              <input
                type="range"
                min={1}
                max={100}
                value={settings.quality.avifQuality}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    quality: {
                      ...current.quality,
                      avifQuality: Number(event.currentTarget.value),
                    },
                  }))
                }
              />
              <small>{settings.quality.avifQuality}</small>
            </label>

            <label>
              <span>PNG 圧縮レベル</span>
              <input
                type="range"
                min={0}
                max={9}
                value={settings.quality.pngCompression}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    quality: {
                      ...current.quality,
                      pngCompression: Number(event.currentTarget.value),
                    },
                  }))
                }
              />
              <small>{settings.quality.pngCompression}</small>
            </label>

            <label>
              <span>GIF 色数</span>
              <input
                type="range"
                min={2}
                max={256}
                value={settings.quality.gifColors}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    quality: {
                      ...current.quality,
                      gifColors: Number(event.currentTarget.value),
                    },
                  }))
                }
              />
              <small>{settings.quality.gifColors}</small>
            </label>

            <label>
              <span>メタデータ</span>
              <select
                value={settings.metadataMode}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    metadataMode: event.currentTarget.value as BatchSettings["metadataMode"],
                  }))
                }
              >
                <option value="strip">削除する</option>
                <option value="keep">保持する</option>
              </select>
              <small>保持はこの MVP ビルドではベストエフォートです。</small>
            </label>

            <div className="checkbox-cluster full-width">
              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={settings.timestamps.preserveCreationTime}
                  onChange={(event) =>
                    setSettings((current) => ({
                      ...current,
                      timestamps: {
                        ...current.timestamps,
                        preserveCreationTime: event.currentTarget.checked,
                      },
                    }))
                  }
                />
                <span>作成日時を引き継ぐ</span>
              </label>
              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={settings.timestamps.preserveLastWriteTime}
                  onChange={(event) =>
                    setSettings((current) => ({
                      ...current,
                      timestamps: {
                        ...current.timestamps,
                        preserveLastWriteTime: event.currentTarget.checked,
                      },
                    }))
                  }
                />
                <span>更新日時を引き継ぐ</span>
              </label>
              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={settings.overwrite}
                  onChange={(event) =>
                    setSettings((current) => ({
                      ...current,
                      overwrite: event.currentTarget.checked,
                    }))
                  }
                />
                <span>上書きを許可する</span>
              </label>
            </div>
          </div>

          <div className="action-row">
            <button type="button" className="primary" disabled={entries.length === 0 || busy} onClick={runBatch}>
              {busy ? "処理中..." : "最適化を実行"}
            </button>
            <button type="button" className="ghost" onClick={() => setResults([])}>
              結果をクリア
            </button>
          </div>

          {errorMessage ? <div className="notice danger">{errorMessage}</div> : null}
        </div>
      </section>

      <section className="panel">
        <div className="panel-header">
          <div>
            <h2>投入一覧</h2>
            <p>フォルダ投入時は再帰探索し、出力でも構造を維持します。</p>
          </div>
        </div>
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>ファイル</th>
                <th>形式</th>
                <th>寸法</th>
                <th>サイズ</th>
                <th>備考</th>
              </tr>
            </thead>
            <tbody>
              {entries.length === 0 ? (
                <tr>
                  <td colSpan={5} className="empty-cell">
                    まだファイルがありません。
                  </td>
                </tr>
              ) : (
                entries.map((entry) => (
                  <tr key={entry.id}>
                    <td>
                      <div className="file-cell">
                        <strong>{entry.fileName}</strong>
                        <small>{entry.sourcePath}</small>
                      </div>
                    </td>
                    <td>{entry.formatLabel}</td>
                    <td>{formatDimension(entry.width, entry.height)}</td>
                    <td>{formatBytes(entry.fileSize)}</td>
                    <td>
                      <div className="tag-list">
                        {entry.animated ? <span className="tag accent">animation</span> : null}
                        {!entry.runtimeSupported ? <span className="tag warning">runtime制約</span> : null}
                        {entry.warnings.map((warning) => (
                          <span key={warning} className="tag subtle">
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

      <section className="results-grid">
        <div className="panel">
          <div className="panel-header">
            <div>
              <h2>進捗</h2>
              <p>1 件失敗しても残りの処理を継続します。</p>
            </div>
          </div>
          <div className="progress-meta">
            <strong>
              {progress.completed} / {progress.total}
            </strong>
            <span>{progress.currentPath ?? "待機中"}</span>
          </div>
          <div className="progress-bar">
            <div
              className="progress-bar-fill"
              style={{
                width: progress.total === 0 ? "0%" : `${Math.round((progress.completed / progress.total) * 100)}%`,
              }}
            />
          </div>
          <div className="summary-grid">
            <div>
              <span>成功</span>
              <strong>{summary.success}</strong>
            </div>
            <div>
              <span>失敗</span>
              <strong>{summary.failed}</strong>
            </div>
            <div>
              <span>Saved</span>
              <strong>{formatBytes(summary.saved)}</strong>
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-header">
            <div>
              <h2>結果</h2>
              <p>Original size / Optimized size / Saved size を一覧表示します。</p>
            </div>
          </div>
          <div className="table-scroll">
            <table>
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
                      まだ実行結果がありません。
                    </td>
                  </tr>
                ) : (
                  results.map((result) => (
                    <tr key={`${result.sourcePath}-${result.outputPath ?? "error"}`}>
                      <td>
                        <div className="file-cell">
                          <strong>{result.sourcePath.split(/[\\/]/).pop()}</strong>
                          <small>{result.sourcePath}</small>
                        </div>
                      </td>
                      <td>
                        <div className="file-cell">
                          <strong>{result.outputFormat ?? "-"}</strong>
                          <small>{result.outputPath ?? result.reason ?? "-"}</small>
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
                            <span key={warning} className="tag subtle">
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
        </div>
      </section>
    </main>
  );
}

export default App;
