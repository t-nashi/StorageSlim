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

/**
 * 出力形式ごとの寸法上限。src-tauri/src/lib.rs の同名定数と一致させること。
 *
 * 実際の判定は Rust 側の check_encoder_dimensions が行う。ここの値は
 * 「実行前に入力一覧へ警告を出す」ための予測にのみ使う。出力形式とリサイズ設定に
 * 依存するため Rust の inspect では判定できず、フロント側で計算している。
 */
export const WEBP_MAX_DIMENSION = 16383;
export const JPEG_MAX_DIMENSION = 65500;
export const GIF_MAX_DIMENSION = 65535;
export const AVIF_MAX_DIMENSION = 65535;
/** 超えても出力はできるが、libavif 系デコーダでは開けなくなる寸法。 */
export const AVIF_VIEWER_MAX_DIMENSION = 32768;

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
  /**
   * 現在処理中のファイル内の進捗率 (0-100)。
   * 動画は 1 件が長いため必要。画像モードでは送られない。
   */
  currentFilePercent?: number | null;
}

// ---------------------------------------------------------------------------
// モード
// ---------------------------------------------------------------------------

/**
 * アプリの動作モード。画像と動画は同一 UI に混在させない
 * （`docs/decision-log.md` の `D-17`）。
 */
export type AppMode = "image" | "video";

// ---------------------------------------------------------------------------
// 動画圧縮モード
// ---------------------------------------------------------------------------

/**
 * 動画の出力形式。
 *
 * `mp4H264` は互換性重視の既定。`webmVp9` は同じ体感画質で小さくなるが、
 * エンコードは遅く、再生できないアプリもある。
 */
export type VideoOutputFormat = "mp4H264" | "webmVp9";

/** 品質はプリセット主体（`D-19`）。CRF は詳細指定でのみ開放する。 */
export type VideoQualityPreset = "high" | "standard" | "small" | "smallest";

/** `reencode` はコンテナに合うコデックへ変換する。MP4 は AAC、WebM は Opus。 */
export type VideoAudioMode = "copy" | "reencode" | "remove";

export interface VideoSettings {
  outputFormat: VideoOutputFormat;
  outputMode: OutputMode;
  customOutputDir: string | null;
  overwrite: boolean;
  resize: ResizeSettings;
  qualityPreset: VideoQualityPreset;
  /** CRF 対応エンコーダのときだけ効く。 */
  crfOverride: number | null;
  fpsLimit: number | null;
  audioMode: VideoAudioMode;
  audioBitrateKbps: number;
  metadataMode: MetadataMode;
  timestamps: TimestampSettings;
  /** 同梱バイナリより優先して使う ffmpeg のパス。 */
  ffmpegPath: string | null;
}

export interface VideoInputEntry {
  id: string;
  sourcePath: string;
  rootPath: string;
  relativePath: string;
  fileName: string;
  formatLabel: string;
  videoCodec: string;
  audioCodec: string | null;
  fileSize: number;
  width: number | null;
  height: number | null;
  durationSec: number | null;
  fps: number | null;
  variableFrameRate: boolean;
  bitRate: number | null;
  rotation: number | null;
  hasAudio: boolean;
  audioTrackCount: number;
  subtitleTrackCount: number;
  hdr: boolean;
  warnings: string[];
}

export interface VideoInspectResponse {
  entries: VideoInputEntry[];
  skipped: SkippedItem[];
  /** 対象外種別の件数。1 件 1 行にすると一覧が埋まるため集約して受け取る。 */
  excludedCount: number;
}

export interface VideoResultItem {
  sourcePath: string;
  outputPath: string | null;
  success: boolean;
  /** 停止操作で打ち切ったもの。失敗とは区別して表示する。 */
  interrupted: boolean;
  outputFormat: string | null;
  originalSize: number;
  optimizedSize: number | null;
  savedSize: number | null;
  savedPercent: number | null;
  width: number | null;
  height: number | null;
  durationSec: number | null;
  /** このファイルの処理にかかった時間 (ms)。コデックごとの速度差を比べるために出す。 */
  elapsedMs: number;
  reason: string | null;
  warnings: string[];
}

export interface VideoProcessResponse {
  results: VideoResultItem[];
}

/** 出力形式ごとの利用可否。使える FFmpeg のビルドによって変わる。 */
export interface VideoFormatSupport {
  format: VideoOutputFormat;
  available: boolean;
  encoder: string | null;
  /** "crf" | "bitrate" */
  rateControl: string | null;
  /** CRF の上限。H.264 は 51、VP9 は 63。 */
  crfMax: number;
  message: string | null;
}

/** 起動時に確認する FFmpeg の利用可否。 */
export interface VideoEnvironment {
  available: boolean;
  ffmpegPath: string | null;
  ffprobePath: string | null;
  version: string | null;
  /** "setting" | "bundled" | "path" */
  source: string | null;
  formats: VideoFormatSupport[];
  message: string | null;
}
