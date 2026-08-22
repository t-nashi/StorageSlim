export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function roundToStep(value: number, min: number, max: number, step: number): number {
  const snapped = Math.round((value - min) / step) * step + min;
  return clamp(Number(snapped.toFixed(4)), min, max);
}

export function formatBytes(bytes: number | null): string {
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

/**
 * 削減量の表示。savedSize が負のとき（出力の方が大きくなったとき）は
 * `+102.6 KB / +766.0%` のように増加であることを明示する。
 */
export function formatSavedDelta(savedSize: number | null, savedPercent: number | null): string {
  if (savedSize == null || savedPercent == null) {
    return "-";
  }
  if (savedSize < 0) {
    return `+${formatBytes(-savedSize)} / +${Math.abs(savedPercent).toFixed(1)}%`;
  }
  return `${formatBytes(savedSize)} / ${savedPercent.toFixed(1)}%`;
}

export function formatDimension(width: number | null, height: number | null): string {
  if (!width || !height) {
    return "-";
  }
  return `${width} x ${height}`;
}

/** 再生時間の表示。1 時間以上は `1:02:03`、それ未満は `2:03`。 */
export function formatDuration(seconds: number | null): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) {
    return "-";
  }
  const total = Math.round(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(secs)}` : `${minutes}:${pad(secs)}`;
}
