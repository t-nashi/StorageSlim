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
}

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
  savedSize: number | null;
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
}
