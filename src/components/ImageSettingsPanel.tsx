import { useState } from "react";
import { ChoiceGroup, type ChoiceOption } from "./ChoiceGroup";
import { InfoHint } from "./InfoHint";
import { QualityField } from "./QualityField";
import { clamp } from "../lib/format";
import {
  isResizeValueMissing,
  imageMetadataOptions,
  resizeModeOptions,
  resizeUnitOptions,
  resizeValueMaxFor,
} from "../lib/settings";
import type { BatchSettings, OutputFormat } from "../types";
import {
  DECODE_LIMIT_DEFAULT_MB,
  DECODE_LIMIT_MAX_MB,
  DECODE_LIMIT_MIN_MB,
} from "../types";

/**
 * 画像圧縮モードの設定パネル。
 *
 * 画像固有の設定項目とその状態更新をここに閉じ込め、モードを増やしたときに
 * App 側の差分が出ないようにしている。
 */
export function ImageSettingsPanel({
  settings,
  updateSettings,
  outputFormatChoices,
  onResetAll,
  clearResizeError,
}: {
  settings: BatchSettings;
  updateSettings: (updater: (current: BatchSettings) => BatchSettings) => void;
  outputFormatChoices: Array<ChoiceOption<OutputFormat>>;
  onResetAll: () => void;
  clearResizeError: () => void;
}) {
  const [advancedExpanded, setAdvancedExpanded] = useState(false);

  const resizeValueDisabled = settings.resize.mode === "none";
  const resizeValueMax = resizeValueMaxFor(settings.resize.unit);
  const resizeValueMissing = isResizeValueMissing(settings.resize);

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
          <span className="field-label">
            出力形式
            <InfoHint
              label="出力形式"
              text="オリジナル維持は入力と同じ形式で書き出します。アニメーション GIF は GIF 出力のみ、AVIF 入力は現ビルドでは読込を制限しています。"
            />
          </span>
          <ChoiceGroup
            value={settings.outputFormat}
            options={outputFormatChoices}
            onChange={(outputFormat) => updateSettings((current) => ({ ...current, outputFormat }))}
          />
        </div>

        <div className="field setting-resize-mode">
          <span className="field-label">
            リサイズ基準
            <InfoHint
              label="リサイズ基準"
              text="縦横比は保ち、指定より大きい入力だけ縮小します。拡大はしません。"
            />
          </span>
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
                          : clamp(Math.round(current.resize.value), 1, unit === "percent" ? 100 : 100000),
                    },
                  }))
                }
              />
            </div>
          </div>
        </div>

        <div className="field setting-metadata">
          <span className="field-label">
            メタデータ
            <InfoHint
              label="メタデータ"
              text="撮影日のみ: 撮影日時と向きだけを残し、GPS などは落とします。保持する: EXIF をそのまま引き継ぎます（XMP / ICC プロファイルは対象外）。GIF / AVIF 出力と HEIC / HEIF 入力では EXIF を引き継げません。"
            />
          </span>
          <ChoiceGroup
            value={settings.metadataMode}
            options={imageMetadataOptions}
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

            <div className="decode-limit">
              <span className="decode-limit-head">
                <label className="decode-limit-label" htmlFor="decode-limit">
                  デコード上限
                </label>
                <InfoHint
                  label="デコード上限"
                  text={`大きな画像の読込に必要な量。上げすぎるとメモリ不足でアプリが終了する場合があります（既定 ${DECODE_LIMIT_DEFAULT_MB} MB / 範囲 ${DECODE_LIMIT_MIN_MB}-${DECODE_LIMIT_MAX_MB}）`}
                />
              </span>
              <input
                id="decode-limit"
                className="decode-limit-input"
                type="number"
                inputMode="numeric"
                min={DECODE_LIMIT_MIN_MB}
                max={DECODE_LIMIT_MAX_MB}
                step={64}
                value={settings.decodeLimitMb}
                onChange={(event) => {
                  const parsed = Number(event.currentTarget.value);
                  if (!Number.isFinite(parsed)) {
                    return;
                  }
                  updateSettings((current) => ({ ...current, decodeLimitMb: Math.round(parsed) }));
                }}
                onBlur={(event) => {
                  const parsed = Number(event.currentTarget.value);
                  const next = Number.isFinite(parsed)
                    ? clamp(Math.round(parsed), DECODE_LIMIT_MIN_MB, DECODE_LIMIT_MAX_MB)
                    : DECODE_LIMIT_DEFAULT_MB;
                  updateSettings((current) => ({ ...current, decodeLimitMb: next }));
                }}
              />
              <span className="decode-limit-unit">MB</span>
            </div>
          </div>
        ) : null}
      </div>
    </aside>
  );
}
