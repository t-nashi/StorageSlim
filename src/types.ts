export type InputFormat =
  | "gif"
  | "jpeg"
  | "png"
  | "webp"
  | "avif"
  | "heic"
  | "heif";

export type OutputFormat =
  | "original"
  | "gif"
  | "jpeg"
  | "png"
  | "webp"
  | "avif";

export type MetadataMode = "strip" | "keep";
export type OutputMode = "desktopDefault" | "custom";
export type ResizeMode = "none" | "width" | "height" | "longEdge";
export type ResizeUnit = "px" | "percent";

export interface InputEntry {
  id: string;
  sourcePath: string;
  rootPath: string;
  relativePath: string;
  fileName: string;
  format: InputFormat;
  formatLabel: string;
  fileSize: number;
  width: number | null;
  height: number | null;
  animated: boolean;
  runtimeSupported: boolean;
  warnings: string[];
}

export interface InspectResponse {
  entries: InputEntry[];
  skipped: SkippedItem[];
}

export interface SkippedItem {
  path: string;
  reason: string;
}

export interface ResizeSettings {
  mode: ResizeMode;
  value: number | null;
  unit: ResizeUnit;
}

export interface QualitySettings {
  jpegQuality: number;
  webpQuality: number;
  avifQuality: number;
  pngCompression: number;
  gifColors: number;
}

export interface TimestampSettings {
  preserveCreationTime: boolean;
  preserveLastWriteTime: boolean;
}

export interface BatchSettings {
  outputFormat: OutputFormat;
  outputMode: OutputMode;
  customOutputDir: string | null;
  overwrite: boolean;
  resize: ResizeSettings;
  quality: QualitySettings;
  metadataMode: MetadataMode;
  timestamps: TimestampSettings;
  /** デコード時に確保を許すメモリ量 (MB)。DECODE_LIMIT_MIN/MAX_MB の範囲。 */
  decodeLimitMb: number;
}

/** デコード上限 (MB) の既定値と範囲。src-tauri/src/lib.rs の定数と一致させること。 */
export const DECODE_LIMIT_DEFAULT_MB = 512;
export const DECODE_LIMIT_MIN_MB = 64;
export const DECODE_LIMIT_MAX_MB = 8192;

export interface ProcessResponse {
  results: ProcessResultItem[];
}

export interface ProcessResultItem {
  sourcePath: string;
  outputPath: string | null;
  success: boolean;
  outputFormat: string | null;
  originalSize: number;
  optimizedSize: number | null;
  /** 削減バイト数。出力の方が大きくなった場合は負の値になる。 */
  savedSize: number | null;
  /** 削減率 (%)。出力の方が大きくなった場合は負の値になる。 */
  savedPercent: number | null;
  width: number | null;
  height: number | null;
  reason: string | null;
  warnings: string[];
}

export interface BatchProgress {
  completed: number;
  total: number;
  currentPath: string | null;
  state?: "running" | "paused" | "stopping";
}
