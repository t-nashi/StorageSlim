import { useState } from "react";
import { ChoiceGroup, type ChoiceOption } from "./ChoiceGroup";
import { clamp } from "../lib/format";
import {
  isResizeValueMissing,
  metadataOptions,
  resizeModeOptions,
  resizeUnitOptions,
  resizeValueMaxFor,
} from "../lib/settings";
import type {
  VideoAudioMode,
  VideoEnvironment,
  VideoQualityPreset,
  VideoSettings,
} from "../types";

const outputFormatOptions: Array<ChoiceOption<VideoSettings["outputFormat"]>> = [
  { value: "mp4H264", label: "MP4 (H.264)" },
];

const qualityOptions: Array<ChoiceOption<VideoQualityPreset>> = [
  { value: "high", label: "高画質" },
  { value: "standard", label: "標準" },
  { value: "small", label: "小さめ" },
  { value: "smallest", label: "最小" },
];

const audioOptions: Array<ChoiceOption<VideoAudioMode>> = [
  { value: "copy", label: "そのままコピー" },
  { value: "aac", label: "AAC で再エンコード" },
  { value: "remove", label: "削除" },
];

const audioBitrateOptions: Array<ChoiceOption<string>> = [
  { value: "96", label: "96k" },
  { value: "128", label: "128k" },
  { value: "192", label: "192k" },
];

const fpsOptions: Array<ChoiceOption<string>> = [
  { value: "none", label: "上限なし" },
  { value: "60", label: "60 fps" },
  { value: "30", label: "30 fps" },
  { value: "24", label: "24 fps" },
];

/** 品質プリセットごとの bits per pixel。Rust 側の QualityPreset と一致させること。 */
const BITS_PER_PIXEL: Record<VideoQualityPreset, number> = {
  high: 0.12,
  standard: 0.08,
  small: 0.05,
  smallest: 0.03,
};

/** CRF 対応エンコーダ向けの値。Rust 側の QualityPreset と一致させること。 */
const PRESET_CRF: Record<VideoQualityPreset, number> = {
  high: 18,
  standard: 23,
  small: 28,
  smallest: 32,
};

/**
 * 1080p30 を 1 分エンコードしたときの目安サイズ (MB)。
 *
 * 動画は結果サイズの予測が画像より難しく、目安がないとプリセットを選べない
 * （`docs/decision-log.md` の `D-19`）。実際の値は素材で大きく変わる。
 */
function estimatedMbPerMinute(preset: VideoQualityPreset): number {
  const bitsPerSecond = 1920 * 1080 * 30 * BITS_PER_PIXEL[preset];
  return (bitsPerSecond * 60) / 8 / 1_000_000;
}

export function VideoSettingsPanel({
  settings,
  updateSettings,
  onResetAll,
  clearResizeError,
  environment,
}: {
  settings: VideoSettings;
  updateSettings: (updater: (current: VideoSettings) => VideoSettings) => void;
  onResetAll: () => void;
  clearResizeError: () => void;
  environment: VideoEnvironment | null;
}) {
  const [advancedExpanded, setAdvancedExpanded] = useState(false);

  const resizeValueDisabled = settings.resize.mode === "none";
  const resizeValueMax = resizeValueMaxFor(settings.resize.unit);
  const resizeValueMissing = isResizeValueMissing(settings.resize);
  const crfSupported = environment?.rateControl === "crf";

  return (
    <aside className="panel settings-panel">
      <div className="panel-header">
        <div className="title-inline">
          <h2>設定</h2>
          <button type="button" className="ghost micro-button" onClick={onResetAll}>
            初期化
          </button>
        </div>
      </div>

      <div className="settings-stack">
        <div className="field setting-output-format">
          <span>出力形式</span>
          <ChoiceGroup
            value={settings.outputFormat}
            options={outputFormatOptions}
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
                  resize: { ...current.resize, mode },
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
                    clearResizeError();
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
                          : clamp(Math.round(current.resize.value), 1, resizeValueMaxFor(unit)),
                    },
                  }))
                }
              />
            </div>
          </div>
          <small className="field-note">寸法は yuv420 の制約で偶数へ丸めます。拡大はしません。</small>
        </div>

        <div className="field setting-quality-preset">
          <span>品質</span>
          <ChoiceGroup
            value={settings.qualityPreset}
            options={qualityOptions}
            onChange={(qualityPreset) => updateSettings((current) => ({ ...current, qualityPreset }))}
          />
          <small className="field-note">
            目安: 1080p30 で 1 分あたり約 {estimatedMbPerMinute(settings.qualityPreset).toFixed(0)} MB
            {crfSupported ? `（CRF ${PRESET_CRF[settings.qualityPreset]} 相当）` : "（ビットレート指定）"}
          </small>
        </div>

        <div className="field setting-fps">
          <span>フレームレート</span>
          <ChoiceGroup
            value={settings.fpsLimit == null ? "none" : String(settings.fpsLimit)}
            options={fpsOptions}
            onChange={(value) =>
              updateSettings((current) => ({
                ...current,
                fpsLimit: value === "none" ? null : Number(value),
              }))
            }
          />
          <small className="field-note">入力より高い値には変換しません。</small>
        </div>

        <div className="field setting-audio">
          <span>音声</span>
          <ChoiceGroup
            value={settings.audioMode}
            options={audioOptions}
            onChange={(audioMode) => updateSettings((current) => ({ ...current, audioMode }))}
          />
          {settings.audioMode === "aac" ? (
            <ChoiceGroup
              value={String(settings.audioBitrateKbps)}
              options={audioBitrateOptions}
              onChange={(value) =>
                updateSettings((current) => ({ ...current, audioBitrateKbps: Number(value) }))
              }
            />
          ) : null}
        </div>

        <div className="field setting-metadata">
          <span>メタデータ</span>
          <ChoiceGroup
            value={settings.metadataMode}
            options={metadataOptions}
            onChange={(metadataMode) => updateSettings((current) => ({ ...current, metadataMode }))}
          />
          <small className="field-note">削除しても向きは保たれます（回転は映像へ焼き込まれます）。</small>
        </div>

        <div className="advanced-toggle">
          <button
            type="button"
            className={`section-disclosure ${advancedExpanded ? "is-open" : ""}`}
            aria-expanded={advancedExpanded}
            onClick={() => setAdvancedExpanded((current) => !current)}
          >
            <span className="section-disclosure-copy">
              <strong>詳細設定</strong>
            </span>
            <span className="section-disclosure-chevron" aria-hidden="true">
              {advancedExpanded ? "▾" : "▸"}
            </span>
          </button>
        </div>

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
                      timestamps: { ...current.timestamps, preserveCreationTime: checked },
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
                      timestamps: { ...current.timestamps, preserveLastWriteTime: checked },
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
                    updateSettings((current) => ({ ...current, overwrite: checked }));
                  }}
                />
                <span>上書きを許可する</span>
              </label>
            </div>

            <div className="decode-limit">
              <label className="decode-limit-label" htmlFor="video-crf">
                CRF 指定
              </label>
              <input
                id="video-crf"
                className="decode-limit-input"
                type="number"
                inputMode="numeric"
                min={0}
                max={51}
                disabled={!crfSupported}
                value={settings.crfOverride ?? ""}
                placeholder="プリセット"
                onChange={(event) => {
                  const rawValue = event.currentTarget.value;
                  if (rawValue === "") {
                    updateSettings((current) => ({ ...current, crfOverride: null }));
                    return;
                  }
                  const parsed = Number(rawValue);
                  if (!Number.isFinite(parsed)) {
                    return;
                  }
                  updateSettings((current) => ({
                    ...current,
                    crfOverride: clamp(Math.round(parsed), 0, 51),
                  }));
                }}
              />
              <span className="decode-limit-unit">CRF</span>
              <small className="decode-limit-hint">
                {crfSupported
                  ? "空欄でプリセットに従う。小さいほど高品質 (0-51)"
                  : `${environment?.videoEncoder ?? "選択中のエンコーダ"} は CRF 指定に対応しないため無効です`}
              </small>
            </div>

            <div className="field">
              <div className="field-inline-head">
                <span>ffmpeg のパス</span>
                <button
                  type="button"
                  className="ghost micro-button"
                  onClick={() => updateSettings((current) => ({ ...current, ffmpegPath: null }))}
                >
                  既定値へ戻す
                </button>
              </div>
              <input
                value={settings.ffmpegPath ?? ""}
                placeholder="空欄なら同梱バイナリ / PATH を使う"
                onChange={(event) => {
                  const raw = event.currentTarget.value;
                  updateSettings((current) => ({
                    ...current,
                    ffmpegPath: raw.trim().length === 0 ? null : raw,
                  }));
                }}
              />
              <small className="field-note">
                {environment?.available
                  ? `${environment.version ?? "ffmpeg"} / エンコーダ ${environment.videoEncoder} (${environment.source})`
                  : (environment?.message ?? "FFmpeg を確認しています...")}
              </small>
            </div>
          </div>
        ) : null}
      </div>
    </aside>
  );
}
