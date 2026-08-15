use std::{
    any::Any,
    collections::HashSet,
    fs::{self, File},
    io::Cursor,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use dirs_next::desktop_dir;
use filetime::{set_file_mtime, FileTime};
use gif::{
    ColorOutput, DecodeOptions, DisposalMethod, Encoder as GifEncoder, Frame as GifFrame, Repeat,
};
use image::{
    codecs::{
        avif::AvifEncoder,
        jpeg::JpegEncoder,
        png::{CompressionType, FilterType, PngEncoder},
    },
    DynamicImage, GenericImageView, ImageEncoder, ImageFormat, ImageReader, Limits, Rgba,
    RgbaImage,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Window};
use walkdir::WalkDir;
use webp::Encoder as WebpEncoder;

#[cfg(windows)]
use std::{
    fs::OpenOptions,
    os::windows::{fs::MetadataExt, io::AsRawHandle},
};
#[cfg(windows)]
use windows_sys::Win32::{Foundation::FILETIME, Storage::FileSystem::SetFileTime};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum InputFormat {
    Gif,
    Jpeg,
    Png,
    Webp,
    Avif,
    Heic,
    Heif,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum OutputFormat {
    Original,
    Gif,
    Jpeg,
    Png,
    Webp,
    Avif,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputEntry {
    id: String,
    source_path: String,
    root_path: String,
    relative_path: String,
    file_name: String,
    format: InputFormat,
    format_label: String,
    file_size: u64,
    width: Option<u32>,
    height: Option<u32>,
    animated: bool,
    runtime_supported: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectResponse {
    entries: Vec<InputEntry>,
    skipped: Vec<SkippedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkippedItem {
    path: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ResizeMode {
    None,
    Width,
    Height,
    LongEdge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ResizeUnit {
    Px,
    Percent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResizeSettings {
    mode: ResizeMode,
    value: Option<u32>,
    unit: ResizeUnit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QualitySettings {
    jpeg_quality: u8,
    webp_quality: f32,
    avif_quality: u8,
    png_compression: u8,
    gif_colors: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum MetadataMode {
    Strip,
    Keep,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimestampSettings {
    preserve_creation_time: bool,
    preserve_last_write_time: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum OutputMode {
    DesktopDefault,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchSettings {
    output_format: OutputFormat,
    output_mode: OutputMode,
    custom_output_dir: Option<String>,
    overwrite: bool,
    resize: ResizeSettings,
    quality: QualitySettings,
    metadata_mode: MetadataMode,
    timestamps: TimestampSettings,
    /// デコード時に確保を許すメモリ量 (MB)。古い設定ファイルには存在しないため既定値を持たせる。
    #[serde(default = "default_decode_limit_mb")]
    decode_limit_mb: u32,
}

fn default_decode_limit_mb() -> u32 {
    DECODE_LIMIT_DEFAULT_MB
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessRequest {
    entries: Vec<InputEntry>,
    settings: BatchSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessResultItem {
    source_path: String,
    output_path: Option<String>,
    success: bool,
    output_format: Option<String>,
    original_size: u64,
    optimized_size: Option<u64>,
    /// 削減バイト数。出力の方が大きくなった場合は負の値になる。
    saved_size: Option<i64>,
    /// 削減率 (%)。出力の方が大きくなった場合は負の値になる。
    saved_percent: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
    reason: Option<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessResponse {
    results: Vec<ProcessResultItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchProgress {
    completed: usize,
    total: usize,
    current_path: Option<String>,
    state: BatchProgressState,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum BatchProgressState {
    Running,
    Paused,
    Stopping,
}

#[derive(Clone, Default)]
struct BatchControl {
    paused: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
}

#[tauri::command]
async fn inspect_inputs(paths: Vec<String>) -> Result<InspectResponse, String> {
    tauri::async_runtime::spawn_blocking(move || inspect_inputs_impl(paths))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
async fn process_batch(
    window: Window,
    control: tauri::State<'_, BatchControl>,
    request: ProcessRequest,
) -> Result<ProcessResponse, String> {
    control.paused.store(false, Ordering::SeqCst);
    control.stop_requested.store(false, Ordering::SeqCst);
    let control = control.inner().clone();

    tauri::async_runtime::spawn_blocking(move || process_batch_impl(window, request, control))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn pause_batch(control: tauri::State<'_, BatchControl>) {
    control.paused.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn resume_batch(control: tauri::State<'_, BatchControl>) {
    control.paused.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn stop_batch(control: tauri::State<'_, BatchControl>) {
    control.stop_requested.store(true, Ordering::SeqCst);
    control.paused.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn get_default_output_dir() -> Result<String, String> {
    default_output_root()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|error| error.to_string())
}

fn inspect_inputs_impl(paths: Vec<String>) -> Result<InspectResponse> {
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = HashSet::new();

    for raw_path in paths {
        let root = PathBuf::from(&raw_path);
        if !root.exists() {
            skipped.push(SkippedItem {
                path: raw_path,
                reason: "パスが存在しません。".to_string(),
            });
            continue;
        }

        if root.is_dir() {
            for dir_entry in WalkDir::new(&root).into_iter().filter_map(|entry| entry.ok()) {
                let path = dir_entry.path();
                if path.is_dir() || is_hidden_or_system(path) {
                    continue;
                }
                let relative = path
                    .strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");
                match inspect_single_safe(path, &root, &relative) {
                    Ok(entry) => {
                        if seen.insert(entry.source_path.clone()) {
                            entries.push(entry);
                        }
                    }
                    Err(error) => skipped.push(SkippedItem {
                        path: path.to_string_lossy().to_string(),
                        reason: format!("{error:#}"),
                    }),
                }
            }
        } else {
            let relative = root
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| root.to_string_lossy().to_string());
            match inspect_single_safe(&root, &root, &relative) {
                Ok(entry) => {
                    if seen.insert(entry.source_path.clone()) {
                        entries.push(entry);
                    }
                }
                Err(error) => skipped.push(SkippedItem {
                    path: raw_path,
                    reason: format!("{error:#}"),
                }),
            }
        }
    }

    entries.sort_by(|a, b| a.source_path.cmp(&b.source_path));
    Ok(InspectResponse { entries, skipped })
}

fn inspect_single_safe(path: &Path, root: &Path, relative: &str) -> Result<InputEntry> {
    catch_task_panic("inspection", || inspect_single(path, root, relative))
}

fn inspect_single(path: &Path, root: &Path, relative: &str) -> Result<InputEntry> {
    let format = detect_input_format(path).ok_or_else(|| anyhow!("unsupported format"))?;
    let metadata = fs::metadata(path).with_context(|| format!("failed to read metadata: {}", path.display()))?;
    let file_size = metadata.len();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| anyhow!("missing file name"))?;

    let mut warnings = Vec::new();
    let (width, height, animated, runtime_supported) = match format {
        InputFormat::Gif => {
            let (frames, gif_width, gif_height) = inspect_gif(path)?;
            if frames > 1 {
                warnings.push("アニメーション保持対象".to_string());
            }
            (Some(gif_width), Some(gif_height), frames > 1, true)
        }
        InputFormat::Heic | InputFormat::Heif => {
            let image = decode_heif_image(path)?;
            warnings.push("HEIC / HEIF は外部 codec 経由で読込します。".to_string());
            (Some(image.width()), Some(image.height()), false, true)
        }
        InputFormat::Avif => {
            warnings.push("AVIF 入力はこのビルドでは一時的に無効化しています。".to_string());
            (None, None, false, false)
        }
        _ => {
            let (w, h) = image::image_dimensions(path)
                .with_context(|| format!("failed to read image dimensions: {}", path.display()))?;
            (Some(w), Some(h), false, true)
        }
    };

    Ok(InputEntry {
        id: path.to_string_lossy().to_string(),
        source_path: path.to_string_lossy().to_string(),
        root_path: root.to_string_lossy().to_string(),
        relative_path: relative.to_string(),
        file_name,
        format_label: format_label(&format).to_string(),
        format,
        file_size,
        width,
        height,
        animated,
        runtime_supported,
        warnings,
    })
}

/// デコード時に確保を許すメモリ量の既定値 (MB)。
/// `image` クレートの `Limits::default().max_alloc` と同じ値。
const DECODE_LIMIT_DEFAULT_MB: u32 = 512;
/// 下げすぎて通常の画像すら通らなくなるのを防ぐための下限 (MB)。
const DECODE_LIMIT_MIN_MB: u32 = 64;
/// 上限 (MB)。これ以上は確保に失敗してプロセスごと落ちる可能性が高い。
const DECODE_LIMIT_MAX_MB: u32 = 8192;

/// UI から渡された MB 値を、実際に使うバイト数へ変換する。
fn decode_limit_bytes(decode_limit_mb: u32) -> u64 {
    u64::from(decode_limit_mb.clamp(DECODE_LIMIT_MIN_MB, DECODE_LIMIT_MAX_MB)) * 1024 * 1024
}

fn catch_task_panic<T, F>(label: &str, task: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    match catch_unwind(AssertUnwindSafe(task)) {
        Ok(result) => result,
        Err(payload) => Err(anyhow!("{label} panicked: {}", panic_payload_to_string(payload))),
    }
}

fn panic_payload_to_string(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn process_batch_impl(window: Window, request: ProcessRequest, control: BatchControl) -> Result<ProcessResponse> {
    let output_root = resolve_output_root(&request.settings)?;
    fs::create_dir_all(&output_root)
        .with_context(|| format!("failed to create output root: {}", output_root.display()))?;

    let total = request.entries.len();
    let mut results = Vec::with_capacity(total);

    for (index, entry) in request.entries.iter().enumerate() {
        if !wait_until_batch_can_continue(&window, &control, index, total, Some(entry.source_path.clone())) {
            break;
        }

        let _ = window.emit(
            "batch-progress",
            BatchProgress {
                completed: index,
                total,
                current_path: Some(entry.source_path.clone()),
                state: BatchProgressState::Running,
            },
        );

        let result = catch_task_panic("processing", || process_one(entry, &request.settings, &output_root))
            .unwrap_or_else(|error| ProcessResultItem {
                source_path: entry.source_path.clone(),
                output_path: None,
                success: false,
                output_format: None,
                original_size: entry.file_size,
                optimized_size: None,
                saved_size: None,
                saved_percent: None,
                width: entry.width,
                height: entry.height,
                // `{:#}` にしないと anyhow の最外殻 context しか出ず、
                // 本当の原因 (例: Memory limit exceeded) が失われる。
                reason: Some(format!("{error:#}")),
                warnings: Vec::new(),
            });
        results.push(result);

        let _ = window.emit(
            "batch-progress",
            BatchProgress {
                completed: index + 1,
                total,
                current_path: Some(entry.source_path.clone()),
                state: BatchProgressState::Running,
            },
        );

        if control.stop_requested.load(Ordering::SeqCst) {
            let _ = window.emit(
                "batch-progress",
                BatchProgress {
                    completed: index + 1,
                    total,
                    current_path: Some(entry.source_path.clone()),
                    state: BatchProgressState::Stopping,
                },
            );
            break;
        }
    }

    Ok(ProcessResponse { results })
}

fn wait_until_batch_can_continue(
    window: &Window,
    control: &BatchControl,
    completed: usize,
    total: usize,
    current_path: Option<String>,
) -> bool {
    while control.paused.load(Ordering::SeqCst) {
        if control.stop_requested.load(Ordering::SeqCst) {
            return false;
        }
        let _ = window.emit(
            "batch-progress",
            BatchProgress {
                completed,
                total,
                current_path: current_path.clone(),
                state: BatchProgressState::Paused,
            },
        );
        std::thread::sleep(Duration::from_millis(120));
    }

    if control.stop_requested.load(Ordering::SeqCst) {
        let _ = window.emit(
            "batch-progress",
            BatchProgress {
                completed,
                total,
                current_path,
                state: BatchProgressState::Stopping,
            },
        );
        return false;
    }

    true
}

fn process_one(entry: &InputEntry, settings: &BatchSettings, output_root: &Path) -> Result<ProcessResultItem> {
    let output_format = resolve_output_format(entry, settings.output_format);
    let output_path = build_output_path(entry, output_root, output_format, settings.overwrite)?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory: {}", parent.display()))?;
    }

    let mut warnings = Vec::new();
    if matches!(settings.metadata_mode, MetadataMode::Keep) {
        warnings.push("メタデータ保持はこの MVP ビルドではベストエフォートです。".to_string());
    }

    if matches!(entry.format, InputFormat::Heic | InputFormat::Heif) && output_format == OutputFormat::Original {
        process_heif_original_copy(entry, settings, &output_path, &mut warnings)?;
    } else if entry.animated {
        if output_format != OutputFormat::Gif {
            return Err(anyhow!(
                "アニメーション GIF は GIF 以外の形式へ変換できません。"
            ));
        }
        process_animated_gif(entry, settings, &output_path, &mut warnings)?;
    } else {
        process_static_image(entry, settings, output_format, &output_path, &mut warnings)?;
    }

    apply_timestamps(entry, settings, &output_path)?;

    let optimized_size = fs::metadata(&output_path)?.len();
    // 出力の方が大きくなる場合があるため、削減量は符号付きで扱う。
    // 0 に丸めると「膨らんだのに削減 0 B」と表示されてしまう。
    let saved_size = entry.file_size as i64 - optimized_size as i64;
    let saved_percent = if entry.file_size == 0 {
        0.0
    } else {
        (saved_size as f64 / entry.file_size as f64) * 100.0
    };
    let (width, height) = read_output_dimensions(entry, output_format, &output_path)
        .unwrap_or((entry.width.unwrap_or(0), entry.height.unwrap_or(0)));

    Ok(ProcessResultItem {
        source_path: entry.source_path.clone(),
        output_path: Some(output_path.to_string_lossy().to_string()),
        success: true,
        output_format: Some(output_format_label(entry, output_format).to_string()),
        original_size: entry.file_size,
        optimized_size: Some(optimized_size),
        saved_size: Some(saved_size),
        saved_percent: Some(saved_percent),
        width: Some(width),
        height: Some(height),
        reason: None,
        warnings,
    })
}

/// libwebp の `WEBP_MAX_DIMENSION`。超えると `VP8_ENC_ERROR_BAD_DIMENSION` でパニックする。
const WEBP_MAX_DIMENSION: u32 = 16_383;
/// rav1e が受け付ける寸法の上限 (`rav1e/src/api/config/mod.rs`)。
const AVIF_MAX_DIMENSION: u32 = 65_535;
/// libavif 系デコーダ (Chrome / Firefox / Windows / IrfanView 等) の既定上限。
/// これを超えた AVIF は生成できるが、多くのビューアで開けない。
const AVIF_VIEWER_MAX_DIMENSION: u32 = 32_768;
/// GIF はヘッダの寸法フィールドが 16bit のため 65535 px までしか表現できない。
const GIF_MAX_DIMENSION: u32 = 65_535;
/// libjpeg の `JPEG_MAX_DIMENSION`。JPEG 規格の SOF フィールドは u16 (65535) だが、
/// libjpeg はオーバーフロー防止のため 65500 で切っている。ほぼ全ての JPEG 読み込み実装が
/// libjpeg 系のため、65501 以上を書くとどこでも開けないファイルになる。
const JPEG_MAX_DIMENSION: u32 = 65_500;

/// エンコーダごとの寸法制限を、実際にエンコードする前に判定する。
///
/// 上限超過はエンコーダ側でパニックしたり無警告で壊れた出力になったりするため、
/// ここで理由の分かるエラーへ変換する。
fn check_encoder_dimensions(
    width: u32,
    height: u32,
    output_format: OutputFormat,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let hard_limit = match output_format {
        OutputFormat::Webp => Some(("WebP", WEBP_MAX_DIMENSION)),
        OutputFormat::Avif => Some(("AVIF", AVIF_MAX_DIMENSION)),
        OutputFormat::Gif => Some(("GIF", GIF_MAX_DIMENSION)),
        OutputFormat::Original | OutputFormat::Jpeg => Some(("JPEG", JPEG_MAX_DIMENSION)),
        OutputFormat::Png => None,
    };

    if let Some((label, limit)) = hard_limit {
        if width > limit || height > limit {
            return Err(anyhow!(
                "{label} は幅・高さとも {limit} px までです（出力予定 {width} x {height}）。リサイズしてから出力してください。"
            ));
        }
    }

    if matches!(output_format, OutputFormat::Avif)
        && (width > AVIF_VIEWER_MAX_DIMENSION || height > AVIF_VIEWER_MAX_DIMENSION)
    {
        // タグとして表示されるため短く保つ。長文にすると 状態 列が横に伸びる。
        warnings.push(format!("主要ビューア上限 {AVIF_VIEWER_MAX_DIMENSION} px 超"));
    }

    Ok(())
}

fn process_static_image(
    entry: &InputEntry,
    settings: &BatchSettings,
    output_format: OutputFormat,
    output_path: &Path,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let image = decode_input_image(entry, settings.decode_limit_mb)?;
    let resized = resize_dynamic_image(image, &settings.resize);
    let (width, height) = resized.dimensions();
    check_encoder_dimensions(width, height, output_format, warnings)?;

    let encoded = encode_static_image(&resized, settings, output_format)?;
    if can_fall_back_to_source_copy(entry, settings, encoded.len() as u64) {
        warnings.push("再圧縮すると大きくなるため元ファイルをコピー".to_string());
        return copy_source_as_output(entry, output_path);
    }

    fs::write(output_path, encoded)
        .with_context(|| format!("failed to write output: {}", output_path.display()))?;

    Ok(())
}

/// エンコード結果をいったんメモリ上に組み立てて返す。
///
/// 出力ファイルはエンコードが成功してから作成する。先にファイルを作ると、
/// エンコード失敗時に 0 byte のファイルが残ってしまうため。
fn encode_static_image(
    source: &DynamicImage,
    settings: &BatchSettings,
    output_format: OutputFormat,
) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();

    match output_format {
        OutputFormat::Original | OutputFormat::Jpeg => {
            let mut encoder =
                JpegEncoder::new_with_quality(&mut buffer, settings.quality.jpeg_quality.clamp(1, 100));
            encoder.encode_image(source)?;
        }
        OutputFormat::Png => {
            let encoder = PngEncoder::new_with_quality(
                &mut buffer,
                png_compression(settings.quality.png_compression),
                FilterType::Adaptive,
            );
            let rgba = source.to_rgba8();
            encoder.write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        }
        OutputFormat::Webp => {
            let rgba = source.to_rgba8();
            let encoded = WebpEncoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
                .encode(settings.quality.webp_quality.clamp(1.0, 100.0));
            buffer.extend_from_slice(&encoded);
        }
        OutputFormat::Avif => {
            let rgba = source.to_rgba8();
            let encoder = AvifEncoder::new_with_speed_quality(
                &mut buffer,
                6,
                settings.quality.avif_quality.clamp(1, 100),
            );
            encoder.write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        }
        OutputFormat::Gif => {
            let rgba = source.to_rgba8();
            encode_static_gif(&rgba, &mut buffer)?;
        }
    }

    Ok(buffer)
}

/// 元ファイルをそのまま出力先へ複製する。失敗時に中途半端な出力を残さない。
fn copy_source_as_output(entry: &InputEntry, output_path: &Path) -> Result<()> {
    if let Err(error) = fs::copy(&entry.source_path, output_path) {
        let _ = fs::remove_file(output_path);
        return Err(error).with_context(|| {
            format!("failed to copy source to output: {}", output_path.display())
        });
    }
    Ok(())
}

/// 再エンコード結果が元より大きい場合に、元ファイルのコピーで代替してよいか判定する。
///
/// 圧縮ツールとして「元より膨らんだ出力」を残す意味はないが、
/// ユーザーが明示した変換 (形式・寸法・メタデータ) を無視してはいけない。
/// コピーで指示を満たせるケースに限って代替する。
fn can_fall_back_to_source_copy(
    entry: &InputEntry,
    settings: &BatchSettings,
    encoded_len: u64,
) -> bool {
    // 元より小さくなっているなら、そのまま出力する。
    if encoded_len <= entry.file_size {
        return false;
    }
    // 形式変換を指定している場合、その形式で出力しなければ指示違反になる。
    if !matches!(settings.output_format, OutputFormat::Original) {
        return false;
    }
    // メタデータ削除は再エンコードによって実現しているため、コピーでは満たせない。
    if matches!(settings.metadata_mode, MetadataMode::Strip) {
        return false;
    }
    // リサイズで寸法が変わる指定なら、コピーでは満たせない。
    let (Some(width), Some(height)) = (entry.width, entry.height) else {
        return false;
    };
    let (target_width, target_height) = resize_target_dimensions(width, height, &settings.resize);
    target_width == width && target_height == height
}

fn process_heif_original_copy(
    entry: &InputEntry,
    settings: &BatchSettings,
    output_path: &Path,
    warnings: &mut Vec<String>,
) -> Result<()> {
    if !matches!(settings.resize.mode, ResizeMode::None) {
        return Err(anyhow!(
            "HEIC / HEIF のオリジナル維持出力ではリサイズ未対応です。別形式へ変換してください。"
        ));
    }

    if matches!(settings.metadata_mode, MetadataMode::Strip) {
        warnings.push("HEIC / HEIF のオリジナル維持出力ではメタデータ削除は未対応です。".to_string());
    }

    warnings.push("HEIC / HEIF のオリジナル維持出力は再圧縮せずコピーします。".to_string());
    copy_source_as_output(entry, output_path)
}

fn process_animated_gif(
    entry: &InputEntry,
    settings: &BatchSettings,
    output_path: &Path,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let file = File::open(&entry.source_path)?;
    let mut decoder = DecodeOptions::new();
    decoder.set_color_output(ColorOutput::RGBA);
    let mut reader = decoder.read_info(file)?;

    // GIF のフレームはキャンバス全体ではなく部分矩形 (left / top / width / height) で
    // 格納されうる。フレーム単体をリサイズすると位置もサイズもずれるため、
    // 一度キャンバスへ合成してから、キャンバスごとリサイズする。
    let canvas_width = u32::from(reader.width());
    let canvas_height = u32::from(reader.height());
    let (target_width, target_height) =
        resize_target_dimensions(canvas_width, canvas_height, &settings.resize);
    let mut canvas = RgbaImage::new(canvas_width, canvas_height);

    // 出力ファイルはエンコードが成功してから作成する (0 byte ファイルを残さないため)。
    let mut buffer = Vec::new();
    {
        let mut encoder =
            GifEncoder::new(&mut buffer, target_width as u16, target_height as u16, &[])?;
        encoder.set_repeat(Repeat::Infinite)?;

        // 合成済みフレームを 1 枚だけ先読みする。
        // 「次フレームでキャンバスを消す必要があるか」が、今のフレームの dispose を決めるため。
        let mut pending: Option<(RgbaImage, u16)> = None;
        // 直前に書き出した結果、画面に出ている状態。差分矩形の基準になる。
        let mut screen: Option<RgbaImage> = None;
        let speed = gif_speed_from_colors(settings.quality.gif_colors);

        while let Some(frame) = reader.read_next_frame()? {
            // Previous 指定のフレームは、描画前の状態を後で復元する必要がある。
            let restore_point = match frame.dispose {
                DisposalMethod::Previous => Some(canvas.clone()),
                _ => None,
            };

            compose_gif_frame(&mut canvas, frame);

            let composed = if target_width == canvas_width && target_height == canvas_height {
                canvas.clone()
            } else {
                image::imageops::resize(
                    &canvas,
                    target_width,
                    target_height,
                    image::imageops::FilterType::Lanczos3,
                )
            };

            if let Some((previous, delay)) = pending.take() {
                // 次が composed なので、previous を書き出す際の dispose を決められる。
                let clear_after = requires_canvas_clear(&previous, &composed);
                write_composed_gif_frame(
                    &mut encoder,
                    &previous,
                    screen.as_ref(),
                    delay,
                    clear_after,
                    speed,
                )?;
                screen = Some(if clear_after {
                    RgbaImage::new(target_width, target_height)
                } else {
                    previous
                });
            }
            pending = Some((composed, frame.delay));

            // 元 GIF の廃棄方法を、次フレームの合成用キャンバスへ反映する。
            match frame.dispose {
                DisposalMethod::Background => clear_gif_frame_rect(&mut canvas, frame),
                DisposalMethod::Previous => {
                    if let Some(restore_point) = restore_point {
                        canvas = restore_point;
                    }
                }
                DisposalMethod::Any | DisposalMethod::Keep => {}
            }
        }

        // 最終フレームには次がないので、キャンバスを消す必要はない。
        if let Some((last, delay)) = pending {
            write_composed_gif_frame(&mut encoder, &last, screen.as_ref(), delay, false, speed)?;
        }
    }

    if can_fall_back_to_source_copy(entry, settings, buffer.len() as u64) {
        warnings.push("再圧縮すると大きくなるため元ファイルをコピー".to_string());
        return copy_source_as_output(entry, output_path);
    }

    fs::write(output_path, buffer)
        .with_context(|| format!("failed to write output: {}", output_path.display()))?;

    Ok(())
}

/// 不透明から透明へ変化した画素があるか。
///
/// GIF の透過は「下を透かす」意味なので、重ね描きでは不透明を透明へ戻せない。
/// その場合は前フレームを Background 廃棄にしてキャンバスを消す必要がある。
fn requires_canvas_clear(current: &RgbaImage, next: &RgbaImage) -> bool {
    current
        .pixels()
        .zip(next.pixels())
        .any(|(now, later)| now[3] != 0 && later[3] == 0)
}

/// 合成済みフレームを、直前の画面との差分矩形として書き出す。
///
/// 全面で書き直すと元 GIF が持っていた差分最適化が失われてサイズが膨らむため、
/// 変化のあった矩形だけを重ねる。
fn write_composed_gif_frame<W: std::io::Write>(
    encoder: &mut GifEncoder<W>,
    target: &RgbaImage,
    screen: Option<&RgbaImage>,
    delay: u16,
    clear_after: bool,
    speed: i32,
) -> Result<()> {
    let dispose = if clear_after {
        DisposalMethod::Background
    } else {
        DisposalMethod::Keep
    };

    // 直前の画面が無い (最初のフレーム) 場合は全面を書く。
    let rect = match screen {
        None => Some((0, 0, target.width(), target.height())),
        Some(screen) => changed_rect(screen, target),
    };

    let Some((left, top, width, height)) = rect else {
        // 変化なし。タイミングだけ維持するための最小フレームを置く。
        let mut raw = [0u8; 4];
        let mut out_frame = GifFrame::from_rgba_speed(1, 1, &mut raw, speed);
        out_frame.delay = delay;
        out_frame.dispose = dispose;
        encoder.write_frame(&out_frame)?;
        return Ok(());
    };

    let cropped = image::imageops::crop_imm(target, left, top, width, height).to_image();
    let mut raw = cropped.into_raw();
    let mut out_frame =
        GifFrame::from_rgba_speed(width as u16, height as u16, raw.as_mut_slice(), speed);
    out_frame.left = left as u16;
    out_frame.top = top as u16;
    out_frame.delay = delay;
    out_frame.dispose = dispose;
    encoder.write_frame(&out_frame)?;
    Ok(())
}

/// 変化のあった画素を囲む最小矩形。変化が無ければ None。
fn changed_rect(screen: &RgbaImage, target: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;

    for y in 0..target.height() {
        for x in 0..target.width() {
            if screen.get_pixel(x, y) == target.get_pixel(x, y) {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }

    if min_x == u32::MAX {
        return None;
    }
    Some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
}

/// 部分矩形フレームを left / top の位置へ合成する。
/// 透明ピクセルは下のフレームを残す (GIF の透過はアルファ合成ではなく素通し)。
fn compose_gif_frame(canvas: &mut RgbaImage, frame: &GifFrame) {
    let left = u32::from(frame.left);
    let top = u32::from(frame.top);
    let width = u32::from(frame.width);
    let height = u32::from(frame.height);

    for y in 0..height {
        let canvas_y = top + y;
        if canvas_y >= canvas.height() {
            break;
        }
        for x in 0..width {
            let canvas_x = left + x;
            if canvas_x >= canvas.width() {
                continue;
            }
            let offset = ((y * width + x) * 4) as usize;
            let Some(pixel) = frame.buffer.get(offset..offset + 4) else {
                continue;
            };
            if pixel[3] == 0 {
                continue;
            }
            canvas.put_pixel(canvas_x, canvas_y, Rgba([pixel[0], pixel[1], pixel[2], pixel[3]]));
        }
    }
}

/// DisposalMethod::Background 用に、そのフレームの矩形だけを透明へ戻す。
fn clear_gif_frame_rect(canvas: &mut RgbaImage, frame: &GifFrame) {
    let left = u32::from(frame.left);
    let top = u32::from(frame.top);
    let width = u32::from(frame.width);
    let height = u32::from(frame.height);

    for y in 0..height {
        let canvas_y = top + y;
        if canvas_y >= canvas.height() {
            break;
        }
        for x in 0..width {
            let canvas_x = left + x;
            if canvas_x >= canvas.width() {
                continue;
            }
            canvas.put_pixel(canvas_x, canvas_y, Rgba([0, 0, 0, 0]));
        }
    }
}

fn encode_static_gif(image: &RgbaImage, out: &mut Vec<u8>) -> Result<()> {
    let mut encoder = GifEncoder::new(out, image.width() as u16, image.height() as u16, &[])?;
    encoder.set_repeat(Repeat::Infinite)?;
    let mut raw = image.clone().into_raw();
    let frame = GifFrame::from_rgba_speed(
        image.width() as u16,
        image.height() as u16,
        raw.as_mut_slice(),
        10,
    );
    encoder.write_frame(&frame)?;
    Ok(())
}

fn decode_input_image(entry: &InputEntry, decode_limit_mb: u32) -> Result<DynamicImage> {
    match entry.format {
        // HEIC / HEIF は heif-oxide 側でデコードするため、この上限は適用されない。
        InputFormat::Heic | InputFormat::Heif => decode_heif_image(Path::new(&entry.source_path)),
        InputFormat::Avif => Err(anyhow!("このビルドでは AVIF 入力の読込を一時停止しています。")),
        _ => {
            let bytes = fs::read(&entry.source_path)?;
            let mut reader =
                ImageReader::with_format(Cursor::new(bytes), image_format_from_input(&entry.format)?);
            let mut limits = Limits::default();
            limits.max_alloc = Some(decode_limit_bytes(decode_limit_mb));
            reader.limits(limits);
            reader.decode().context("failed to decode image")
        }
    }
}

fn decode_avif_image(path: &Path) -> Result<DynamicImage> {
    heif_oxide::decode_file(path)
        .with_context(|| format!("failed to decode AVIF image: {}", path.display()))
        .and_then(|decoded| {
            let rgba = decoded.to_rgba8();
            let image = RgbaImage::from_raw(decoded.width, decoded.height, rgba)
                .ok_or_else(|| anyhow!("failed to materialize AVIF RGBA buffer"))?;
            Ok(DynamicImage::ImageRgba8(image))
        })
}

fn decode_heif_image(path: &Path) -> Result<DynamicImage> {
    let decoded = heif_oxide::decode_file(path)
        .with_context(|| format!("failed to decode HEIC / HEIF image: {}", path.display()))?;
    let rgba = decoded.to_rgba8();
    let image = RgbaImage::from_raw(decoded.width, decoded.height, rgba)
        .ok_or_else(|| anyhow!("failed to materialize HEIC / HEIF RGBA buffer"))?;
    Ok(DynamicImage::ImageRgba8(image))
}

fn read_output_dimensions(entry: &InputEntry, output_format: OutputFormat, path: &Path) -> Result<(u32, u32)> {
    match output_format {
        OutputFormat::Original if matches!(entry.format, InputFormat::Heic | InputFormat::Heif) => {
            Ok((entry.width.unwrap_or(0), entry.height.unwrap_or(0)))
        }
        OutputFormat::Avif => {
            let image = decode_avif_image(path)?;
            Ok(image.dimensions())
        }
        _ => image::image_dimensions(path).map_err(Into::into),
    }
}

fn resolve_output_root(settings: &BatchSettings) -> Result<PathBuf> {
    match settings.output_mode {
        OutputMode::DesktopDefault => default_output_root(),
        OutputMode::Custom => match settings.custom_output_dir.as_deref() {
            Some(path) if !path.trim().is_empty() => Ok(PathBuf::from(path)),
            _ => default_output_root(),
        },
    }
}

fn default_output_root() -> Result<PathBuf> {
    let desktop = desktop_dir().ok_or_else(|| anyhow!("desktop directory not available"))?;
    Ok(desktop.join("@StorageSlim").join("output"))
}

fn resolve_output_format(entry: &InputEntry, requested: OutputFormat) -> OutputFormat {
    match requested {
        OutputFormat::Original => match entry.format {
            InputFormat::Gif => OutputFormat::Gif,
            InputFormat::Jpeg => OutputFormat::Jpeg,
            InputFormat::Png => OutputFormat::Png,
            InputFormat::Webp => OutputFormat::Webp,
            InputFormat::Avif => OutputFormat::Avif,
            InputFormat::Heic | InputFormat::Heif => OutputFormat::Original,
        },
        other => other,
    }
}

fn build_output_path(
    entry: &InputEntry,
    output_root: &Path,
    output_format: OutputFormat,
    overwrite: bool,
) -> Result<PathBuf> {
    let extension = output_extension(entry, output_format);
    let relative_path = Path::new(&entry.relative_path);
    let stem = relative_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .ok_or_else(|| anyhow!("missing output file stem"))?;
    let parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
    let mut candidate = output_root.join(parent).join(format!("{stem}.{extension}"));
    if overwrite {
        return Ok(candidate);
    }
    let mut counter = 1u32;
    while candidate.exists() {
        candidate = output_root.join(parent).join(format!("{stem}_{counter}.{extension}"));
        counter += 1;
    }
    Ok(candidate)
}

fn resize_dynamic_image(image: DynamicImage, settings: &ResizeSettings) -> DynamicImage {
    let (width, height) = image.dimensions();
    let (target_width, target_height) = resize_target_dimensions(width, height, settings);
    if target_width == width && target_height == height {
        return image;
    }
    image.resize(target_width, target_height, image::imageops::FilterType::Lanczos3)
}

/// リサイズ後の寸法だけを求める。
///
/// アニメーション GIF では、フレームを描き込む前にキャンバスの出力寸法を確定させる
/// 必要があるため、実際のリサイズ処理と分離している。
fn resize_target_dimensions(width: u32, height: u32, settings: &ResizeSettings) -> (u32, u32) {
    let Some(value) = settings.value else {
        return (width, height);
    };
    let scaled_value = match settings.unit {
        ResizeUnit::Px => value,
        ResizeUnit::Percent => {
            let basis = match settings.mode {
                ResizeMode::None => return (width, height),
                ResizeMode::Width => width,
                ResizeMode::Height => height,
                ResizeMode::LongEdge => width.max(height),
            };
            ((basis as f64 * (value as f64 / 100.0)).round() as u32).max(1)
        }
    };
    let (target_width, target_height) = match settings.mode {
        ResizeMode::None => return (width, height),
        ResizeMode::Width => {
            let new_width = scaled_value.min(width).max(1);
            let new_height = ((height as f64 * (new_width as f64 / width as f64)).round() as u32).max(1);
            (new_width, new_height)
        }
        ResizeMode::Height => {
            let new_height = scaled_value.min(height).max(1);
            let new_width = ((width as f64 * (new_height as f64 / height as f64)).round() as u32).max(1);
            (new_width, new_height)
        }
        ResizeMode::LongEdge => {
            let limited = scaled_value.min(width.max(height)).max(1);
            if width >= height {
                let new_width = limited;
                let new_height = ((height as f64 * (new_width as f64 / width as f64)).round() as u32).max(1);
                (new_width, new_height)
            } else {
                let new_height = limited;
                let new_width = ((width as f64 * (new_height as f64 / height as f64)).round() as u32).max(1);
                (new_width, new_height)
            }
        }
    };

    // 拡大はしない。
    if target_width >= width && target_height >= height {
        return (width, height);
    }

    (target_width, target_height)
}

fn inspect_gif(path: &Path) -> Result<(usize, u32, u32)> {
    let file = File::open(path)?;
    let mut decoder = DecodeOptions::new();
    decoder.set_color_output(ColorOutput::RGBA);
    let mut reader = decoder.read_info(file)?;
    let width = reader.width() as u32;
    let height = reader.height() as u32;
    let mut frames = 0usize;
    while reader.read_next_frame()?.is_some() {
        frames += 1;
        if frames > 1 {
            break;
        }
    }
    if frames == 0 {
        frames = 1;
    }
    Ok((frames, width, height))
}

fn detect_input_format(path: &Path) -> Option<InputFormat> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match extension.as_str() {
        "gif" => Some(InputFormat::Gif),
        "jpg" | "jpeg" => Some(InputFormat::Jpeg),
        "png" => Some(InputFormat::Png),
        "webp" => Some(InputFormat::Webp),
        "avif" => Some(InputFormat::Avif),
        "heic" => Some(InputFormat::Heic),
        "heif" => Some(InputFormat::Heif),
        _ => None,
    }
}

fn image_format_from_input(format: &InputFormat) -> Result<ImageFormat> {
    match format {
        InputFormat::Gif => Ok(ImageFormat::Gif),
        InputFormat::Jpeg => Ok(ImageFormat::Jpeg),
        InputFormat::Png => Ok(ImageFormat::Png),
        InputFormat::Webp => Ok(ImageFormat::WebP),
        InputFormat::Avif => Ok(ImageFormat::Avif),
        InputFormat::Heic | InputFormat::Heif => Err(anyhow!("HEIC / HEIF uses external decoder path")),
    }
}

fn output_extension(entry: &InputEntry, format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Original => match entry.format {
            InputFormat::Heic => "heic",
            InputFormat::Heif => "heif",
            _ => "bin",
        },
        OutputFormat::Gif => "gif",
        OutputFormat::Jpeg => "jpg",
        OutputFormat::Png => "png",
        OutputFormat::Webp => "webp",
        OutputFormat::Avif => "avif",
    }
}

fn output_format_label(entry: &InputEntry, format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Original => format_label(&entry.format),
        OutputFormat::Gif => "GIF",
        OutputFormat::Jpeg => "JPEG",
        OutputFormat::Png => "PNG",
        OutputFormat::Webp => "WebP",
        OutputFormat::Avif => "AVIF",
    }
}

fn format_label(format: &InputFormat) -> &'static str {
    match format {
        InputFormat::Gif => "GIF",
        InputFormat::Jpeg => "JPEG",
        InputFormat::Png => "PNG",
        InputFormat::Webp => "WebP",
        InputFormat::Avif => "AVIF",
        InputFormat::Heic => "HEIC",
        InputFormat::Heif => "HEIF",
    }
}

fn png_compression(level: u8) -> CompressionType {
    match level {
        0..=2 => CompressionType::Fast,
        3..=6 => CompressionType::Default,
        _ => CompressionType::Best,
    }
}

fn gif_speed_from_colors(colors: u16) -> i32 {
    if colors <= 64 {
        1
    } else if colors <= 128 {
        5
    } else {
        10
    }
}

fn apply_timestamps(entry: &InputEntry, settings: &BatchSettings, output_path: &Path) -> Result<()> {
    let metadata = fs::metadata(&entry.source_path)?;
    if settings.timestamps.preserve_last_write_time {
        if let Ok(modified) = metadata.modified() {
            set_file_mtime(output_path, FileTime::from_system_time(modified))?;
        }
    }

    #[cfg(windows)]
    if settings.timestamps.preserve_creation_time {
        if let Ok(created) = metadata.created() {
            set_windows_creation_time(output_path, created)?;
        }
    }

    Ok(())
}

#[cfg(windows)]
fn set_windows_creation_time(path: &Path, created: SystemTime) -> Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    let handle = file.as_raw_handle() as *mut std::ffi::c_void;
    let creation = system_time_to_filetime(created);
    let result = unsafe { SetFileTime(handle, &creation, std::ptr::null(), std::ptr::null()) };
    if result == 0 {
        return Err(anyhow!("failed to set creation time"));
    }
    Ok(())
}

#[cfg(windows)]
fn system_time_to_filetime(time: SystemTime) -> FILETIME {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let intervals = (duration.as_secs() + 11_644_473_600) * 10_000_000
        + u64::from(duration.subsec_nanos() / 100);
    FILETIME {
        dwLowDateTime: intervals as u32,
        dwHighDateTime: (intervals >> 32) as u32,
    }
}

#[cfg(not(windows))]
fn is_hidden_or_system(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_hidden_or_system(path: &Path) -> bool {
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    let dotfile = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false);
    if dotfile {
        return true;
    }
    fs::metadata(path)
        .map(|metadata| {
            let attributes = metadata.file_attributes();
            attributes & FILE_ATTRIBUTE_HIDDEN != 0 || attributes & FILE_ATTRIBUTE_SYSTEM != 0
        })
        .unwrap_or(false)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(BatchControl::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            inspect_inputs,
            process_batch,
            pause_batch,
            resume_batch,
            stop_batch,
            get_default_output_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod sample_debug;

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("storageslim-{name}-{}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path).unwrap();
        }
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn inspect_png_file() {
        let dir = temp_dir("inspect");
        let path = dir.join("sample.png");
        let image = ImageBuffer::<Rgba<u8>, _>::from_pixel(10, 8, Rgba([255, 0, 0, 255]));
        image.save(&path).unwrap();

        let response = inspect_inputs_impl(vec![path.to_string_lossy().to_string()]).unwrap();
        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].width, Some(10));
        assert_eq!(response.entries[0].height, Some(8));
    }

    #[test]
    fn process_png_to_jpeg() {
        let dir = temp_dir("process-jpeg");
        let input = dir.join("photo.png");
        let image = ImageBuffer::<Rgba<u8>, _>::from_pixel(40, 30, Rgba([0, 120, 255, 255]));
        image.save(&input).unwrap();

        let inspect = inspect_inputs_impl(vec![input.to_string_lossy().to_string()]).unwrap();
        let entry = inspect.entries[0].clone();
        let settings = BatchSettings {
            output_format: OutputFormat::Jpeg,
            output_mode: OutputMode::Custom,
            custom_output_dir: Some(dir.join("output").to_string_lossy().to_string()),
            overwrite: false,
            resize: ResizeSettings {
                mode: ResizeMode::Width,
                value: Some(20),
                unit: ResizeUnit::Px,
            },
            quality: QualitySettings {
                jpeg_quality: 80,
                webp_quality: 80.0,
                avif_quality: 50,
                png_compression: 6,
                gif_colors: 128,
            },
            metadata_mode: MetadataMode::Strip,
            timestamps: TimestampSettings {
                preserve_creation_time: false,
                preserve_last_write_time: false,
            },
            decode_limit_mb: DECODE_LIMIT_DEFAULT_MB,
        };
        let output_root = resolve_output_root(&settings).unwrap();
        fs::create_dir_all(&output_root).unwrap();

        let result = process_one(&entry, &settings, &output_root).unwrap();
        assert!(result.success);
        assert!(result.output_path.unwrap().ends_with(".jpg"));
    }

    #[test]
    fn animated_gif_cannot_convert_to_png() {
        let entry = InputEntry {
            id: "gif".to_string(),
            source_path: "dummy.gif".to_string(),
            root_path: "dummy.gif".to_string(),
            relative_path: "dummy.gif".to_string(),
            file_name: "dummy.gif".to_string(),
            format: InputFormat::Gif,
            format_label: "GIF".to_string(),
            file_size: 1,
            width: Some(1),
            height: Some(1),
            animated: true,
            runtime_supported: true,
            warnings: vec![],
        };
        let settings = BatchSettings {
            output_format: OutputFormat::Png,
            output_mode: OutputMode::Custom,
            custom_output_dir: Some(temp_dir("anim-error").to_string_lossy().to_string()),
            overwrite: false,
            resize: ResizeSettings {
                mode: ResizeMode::None,
                value: None,
                unit: ResizeUnit::Px,
            },
            quality: QualitySettings {
                jpeg_quality: 80,
                webp_quality: 80.0,
                avif_quality: 50,
                png_compression: 6,
                gif_colors: 128,
            },
            metadata_mode: MetadataMode::Strip,
            timestamps: TimestampSettings {
                preserve_creation_time: false,
                preserve_last_write_time: false,
            },
            decode_limit_mb: DECODE_LIMIT_DEFAULT_MB,
        };
        let output_root = resolve_output_root(&settings).unwrap();
        let error = process_one(&entry, &settings, &output_root).unwrap_err();
        assert!(error.to_string().contains("アニメーション GIF"));
    }

    #[test]
    fn resize_percent_uses_relative_dimensions() {
        let image =
            DynamicImage::ImageRgba8(ImageBuffer::<Rgba<u8>, _>::from_pixel(400, 200, Rgba([0, 0, 0, 255])));

        let resized = resize_dynamic_image(
            image,
            &ResizeSettings {
                mode: ResizeMode::LongEdge,
                value: Some(50),
                unit: ResizeUnit::Percent,
            },
        );

        assert_eq!(resized.dimensions(), (200, 100));
    }

    #[test]
    fn webp_dimension_limit_is_enforced() {
        let mut warnings = Vec::new();
        assert!(check_encoder_dimensions(16383, 64, OutputFormat::Webp, &mut warnings).is_ok());
        assert!(check_encoder_dimensions(64, 16383, OutputFormat::Webp, &mut warnings).is_ok());

        let error = check_encoder_dimensions(16384, 64, OutputFormat::Webp, &mut warnings).unwrap_err();
        assert!(error.to_string().contains("16383"));
        assert!(check_encoder_dimensions(64, 16384, OutputFormat::Webp, &mut warnings).is_err());
        assert!(warnings.is_empty());
    }

    #[test]
    fn avif_over_viewer_limit_warns_but_succeeds() {
        let mut warnings = Vec::new();
        assert!(check_encoder_dimensions(32768, 64, OutputFormat::Avif, &mut warnings).is_ok());
        assert!(warnings.is_empty());

        assert!(check_encoder_dimensions(32769, 64, OutputFormat::Avif, &mut warnings).is_ok());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("32768"));

        assert!(check_encoder_dimensions(65536, 64, OutputFormat::Avif, &mut warnings).is_err());
    }

    #[test]
    fn gif_dimension_limit_is_enforced() {
        let mut warnings = Vec::new();
        assert!(check_encoder_dimensions(65535, 1, OutputFormat::Gif, &mut warnings).is_ok());
        assert!(check_encoder_dimensions(65536, 1, OutputFormat::Gif, &mut warnings).is_err());
    }

    #[test]
    fn jpeg_dimension_limit_follows_libjpeg() {
        // 規格上は 65535 まで書けるが、libjpeg 系デコーダは 65500 までしか読めない。
        let mut warnings = Vec::new();
        assert!(check_encoder_dimensions(65500, 64, OutputFormat::Jpeg, &mut warnings).is_ok());

        let error = check_encoder_dimensions(65501, 64, OutputFormat::Jpeg, &mut warnings).unwrap_err();
        assert!(error.to_string().contains("65500"));
        assert!(check_encoder_dimensions(64, 65501, OutputFormat::Jpeg, &mut warnings).is_err());
    }

    /// 40x20 のキャンバスに、2 コマ目だけ部分矩形 (10,5 から 8x6) を持つ GIF を作る。
    fn write_animated_gif(path: &Path) {
        let mut file = File::create(path).unwrap();
        let mut encoder = GifEncoder::new(&mut file, 40, 20, &[]).unwrap();
        encoder.set_repeat(Repeat::Infinite).unwrap();

        let mut full = vec![0u8; 40 * 20 * 4];
        for pixel in full.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[20, 140, 255, 255]);
        }
        let mut first = GifFrame::from_rgba_speed(40, 20, &mut full, 10);
        first.delay = 10;
        encoder.write_frame(&first).unwrap();

        let mut patch = vec![0u8; 8 * 6 * 4];
        for pixel in patch.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 80, 0, 255]);
        }
        let mut second = GifFrame::from_rgba_speed(8, 6, &mut patch, 10);
        second.left = 10;
        second.top = 5;
        second.delay = 10;
        encoder.write_frame(&second).unwrap();
    }

    #[test]
    fn animated_gif_resize_keeps_canvas_and_frames_in_sync() {
        let dir = temp_dir("anim-resize");
        let input = dir.join("anim.gif");
        write_animated_gif(&input);

        let inspect = inspect_inputs_impl(vec![input.to_string_lossy().to_string()]).unwrap();
        let entry = inspect.entries[0].clone();
        assert!(entry.animated);

        let settings = BatchSettings {
            output_format: OutputFormat::Gif,
            output_mode: OutputMode::Custom,
            custom_output_dir: Some(dir.join("output").to_string_lossy().to_string()),
            overwrite: false,
            resize: ResizeSettings {
                mode: ResizeMode::LongEdge,
                value: Some(50),
                unit: ResizeUnit::Percent,
            },
            quality: QualitySettings {
                jpeg_quality: 80,
                webp_quality: 80.0,
                avif_quality: 50,
                png_compression: 6,
                gif_colors: 128,
            },
            metadata_mode: MetadataMode::Strip,
            timestamps: TimestampSettings {
                preserve_creation_time: false,
                preserve_last_write_time: false,
            },
            decode_limit_mb: DECODE_LIMIT_DEFAULT_MB,
        };
        let output_root = resolve_output_root(&settings).unwrap();
        fs::create_dir_all(&output_root).unwrap();

        let result = process_one(&entry, &settings, &output_root).unwrap();
        assert!(result.success);

        // 論理スクリーンサイズがリサイズ後の寸法になっていること。
        // (以前はここが元サイズ 40x20 のまま、フレームだけ 20x10 になっていた)
        let output_path = PathBuf::from(result.output_path.unwrap());
        let (frames, canvas_width, canvas_height) = inspect_gif(&output_path).unwrap();
        assert_eq!((canvas_width, canvas_height), (20, 10));
        assert_eq!(frames, 2);

        // 先頭フレームは全面、以降は差分矩形として書かれていること。
        // (全面のまま書くと元 GIF の差分最適化が失われてサイズが膨らむ)
        let file = File::open(&output_path).unwrap();
        let mut options = DecodeOptions::new();
        options.set_color_output(ColorOutput::RGBA);
        let mut reader = options.read_info(file).unwrap();

        let first = reader.read_next_frame().unwrap().unwrap();
        assert_eq!((first.width, first.height), (20, 10));
        assert_eq!((first.left, first.top), (0, 0));

        let second = reader.read_next_frame().unwrap().unwrap();
        assert!(
            second.width < 20 || second.height < 10 || second.left > 0 || second.top > 0,
            "2 コマ目が全面のまま: {}x{}+{}+{}",
            second.width,
            second.height,
            second.left,
            second.top
        );
    }

    #[test]
    fn changed_rect_finds_minimal_bounds() {
        let mut screen = RgbaImage::from_pixel(10, 10, Rgba([0, 0, 0, 255]));
        assert_eq!(changed_rect(&screen, &screen.clone()), None);

        let mut target = screen.clone();
        target.put_pixel(3, 4, Rgba([255, 0, 0, 255]));
        target.put_pixel(5, 6, Rgba([255, 0, 0, 255]));
        assert_eq!(changed_rect(&screen, &target), Some((3, 4, 3, 3)));

        // 不透明 -> 透明 はキャンバス消去が必要と判定されること。
        assert!(!requires_canvas_clear(&screen, &target));
        screen.put_pixel(0, 0, Rgba([9, 9, 9, 255]));
        let mut cleared = screen.clone();
        cleared.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        assert!(requires_canvas_clear(&screen, &cleared));
    }

    #[test]
    fn decode_limit_is_clamped_to_supported_range() {
        assert_eq!(decode_limit_bytes(DECODE_LIMIT_DEFAULT_MB), 512 * 1024 * 1024);
        // 範囲外の指定はクランプされる。設定ファイルを直接書き換えられても壊れない。
        assert_eq!(decode_limit_bytes(0), decode_limit_bytes(DECODE_LIMIT_MIN_MB));
        assert_eq!(decode_limit_bytes(99_999), decode_limit_bytes(DECODE_LIMIT_MAX_MB));
    }

    #[test]
    fn size_increase_is_reported_as_negative() {
        let dir = temp_dir("size-increase");
        let input = dir.join("tiny.png");
        // 単色 8x8 の PNG は数十バイト。JPEG はヘッダだけで数百バイトになるため必ず増える。
        let image = ImageBuffer::<Rgba<u8>, _>::from_pixel(8, 8, Rgba([1, 2, 3, 255]));
        image.save(&input).unwrap();

        let inspect = inspect_inputs_impl(vec![input.to_string_lossy().to_string()]).unwrap();
        let entry = inspect.entries[0].clone();
        let settings = BatchSettings {
            output_format: OutputFormat::Jpeg,
            output_mode: OutputMode::Custom,
            custom_output_dir: Some(dir.join("output").to_string_lossy().to_string()),
            overwrite: false,
            resize: ResizeSettings {
                mode: ResizeMode::None,
                value: None,
                unit: ResizeUnit::Px,
            },
            quality: QualitySettings {
                jpeg_quality: 80,
                webp_quality: 80.0,
                avif_quality: 50,
                png_compression: 6,
                gif_colors: 128,
            },
            metadata_mode: MetadataMode::Strip,
            timestamps: TimestampSettings {
                preserve_creation_time: false,
                preserve_last_write_time: false,
            },
            decode_limit_mb: DECODE_LIMIT_DEFAULT_MB,
        };
        let output_root = resolve_output_root(&settings).unwrap();
        fs::create_dir_all(&output_root).unwrap();

        let result = process_one(&entry, &settings, &output_root).unwrap();
        assert!(result.success);
        assert!(result.optimized_size.unwrap() > result.original_size);
        // 0 に丸めず、増加を負の値として報告する。
        assert!(result.saved_size.unwrap() < 0);
        assert!(result.saved_percent.unwrap() < 0.0);
    }

    #[test]
    fn oversized_webp_leaves_no_output_file() {
        let dir = temp_dir("webp-limit");
        let input = dir.join("wide.png");
        let image = ImageBuffer::<Rgba<u8>, _>::from_pixel(16384, 8, Rgba([10, 20, 30, 255]));
        image.save(&input).unwrap();

        let inspect = inspect_inputs_impl(vec![input.to_string_lossy().to_string()]).unwrap();
        let entry = inspect.entries[0].clone();
        let settings = BatchSettings {
            output_format: OutputFormat::Webp,
            output_mode: OutputMode::Custom,
            custom_output_dir: Some(dir.join("output").to_string_lossy().to_string()),
            overwrite: false,
            resize: ResizeSettings {
                mode: ResizeMode::None,
                value: None,
                unit: ResizeUnit::Px,
            },
            quality: QualitySettings {
                jpeg_quality: 80,
                webp_quality: 80.0,
                avif_quality: 50,
                png_compression: 6,
                gif_colors: 128,
            },
            metadata_mode: MetadataMode::Strip,
            timestamps: TimestampSettings {
                preserve_creation_time: false,
                preserve_last_write_time: false,
            },
            decode_limit_mb: DECODE_LIMIT_DEFAULT_MB,
        };
        let output_root = resolve_output_root(&settings).unwrap();
        fs::create_dir_all(&output_root).unwrap();

        let error = process_one(&entry, &settings, &output_root).unwrap_err();
        assert!(error.to_string().contains("16383"));
        // 失敗時に 0 byte ファイルを残さない。
        assert!(!output_root.join("wide.webp").exists());
    }
}
