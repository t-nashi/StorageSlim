use std::{
    collections::HashSet,
    fs::{self, File},
    io::BufWriter,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use dirs_next::desktop_dir;
use filetime::{set_file_mtime, FileTime};
use gif::{ColorOutput, DecodeOptions, Encoder as GifEncoder, Frame as GifFrame, Repeat};
use image::{
    codecs::{
        avif::AvifEncoder,
        jpeg::JpegEncoder,
        png::{CompressionType, FilterType, PngEncoder},
    },
    DynamicImage, GenericImageView, ImageEncoder, ImageFormat, RgbaImage,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Window};
use walkdir::WalkDir;
use webp::Encoder as WebpEncoder;
use zenpixels::PixelDescriptor;

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
struct ResizeSettings {
    mode: ResizeMode,
    value: Option<u32>,
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
    saved_size: Option<u64>,
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
}

#[tauri::command]
fn inspect_inputs(paths: Vec<String>) -> Result<InspectResponse, String> {
    inspect_inputs_impl(paths).map_err(|error| error.to_string())
}

#[tauri::command]
fn process_batch(window: Window, request: ProcessRequest) -> Result<ProcessResponse, String> {
    process_batch_impl(window, request).map_err(|error| error.to_string())
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
                match inspect_single(path, &root, &relative) {
                    Ok(entry) => {
                        if seen.insert(entry.source_path.clone()) {
                            entries.push(entry);
                        }
                    }
                    Err(error) => skipped.push(SkippedItem {
                        path: path.to_string_lossy().to_string(),
                        reason: error.to_string(),
                    }),
                }
            }
        } else {
            let relative = root
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| root.to_string_lossy().to_string());
            match inspect_single(&root, &root, &relative) {
                Ok(entry) => {
                    if seen.insert(entry.source_path.clone()) {
                        entries.push(entry);
                    }
                }
                Err(error) => skipped.push(SkippedItem {
                    path: raw_path,
                    reason: error.to_string(),
                }),
            }
        }
    }

    entries.sort_by(|a, b| a.source_path.cmp(&b.source_path));
    Ok(InspectResponse { entries, skipped })
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
            let image = decode_avif_image(path)?;
            (Some(image.width()), Some(image.height()), false, true)
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

fn process_batch_impl(window: Window, request: ProcessRequest) -> Result<ProcessResponse> {
    let output_root = resolve_output_root(&request.settings)?;
    fs::create_dir_all(&output_root)
        .with_context(|| format!("failed to create output root: {}", output_root.display()))?;

    let total = request.entries.len();
    let mut results = Vec::with_capacity(total);

    for (index, entry) in request.entries.iter().enumerate() {
        let _ = window.emit(
            "batch-progress",
            BatchProgress {
                completed: index,
                total,
                current_path: Some(entry.source_path.clone()),
            },
        );

        let result = process_one(entry, &request.settings, &output_root).unwrap_or_else(|error| ProcessResultItem {
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
            reason: Some(error.to_string()),
            warnings: Vec::new(),
        });
        results.push(result);

        let _ = window.emit(
            "batch-progress",
            BatchProgress {
                completed: index + 1,
                total,
                current_path: Some(entry.source_path.clone()),
            },
        );
    }

    Ok(ProcessResponse { results })
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
        process_animated_gif(entry, settings, &output_path)?;
    } else {
        process_static_image(entry, settings, output_format, &output_path)?;
    }

    apply_timestamps(entry, settings, &output_path)?;

    let optimized_size = fs::metadata(&output_path)?.len();
    let saved_size = entry.file_size.saturating_sub(optimized_size);
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

fn process_static_image(
    entry: &InputEntry,
    settings: &BatchSettings,
    output_format: OutputFormat,
    output_path: &Path,
) -> Result<()> {
    let image = decode_input_image(entry)?;
    let resized = resize_dynamic_image(image, &settings.resize);

    match output_format {
        OutputFormat::Original | OutputFormat::Jpeg => {
            let mut writer = BufWriter::new(File::create(output_path)?);
            let mut encoder =
                JpegEncoder::new_with_quality(&mut writer, settings.quality.jpeg_quality.clamp(1, 100));
            encoder.encode_image(&resized)?;
        }
        OutputFormat::Png => {
            let mut writer = BufWriter::new(File::create(output_path)?);
            let encoder = PngEncoder::new_with_quality(
                &mut writer,
                png_compression(settings.quality.png_compression),
                FilterType::Adaptive,
            );
            let rgba = resized.to_rgba8();
            encoder.write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        }
        OutputFormat::Webp => {
            let rgba = resized.to_rgba8();
            let encoded = WebpEncoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
                .encode(settings.quality.webp_quality.clamp(1.0, 100.0));
            fs::write(output_path, encoded.to_vec())?;
        }
        OutputFormat::Avif => {
            let mut writer = BufWriter::new(File::create(output_path)?);
            let rgba = resized.to_rgba8();
            let encoder = AvifEncoder::new_with_speed_quality(
                &mut writer,
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
            let rgba = resized.to_rgba8();
            encode_static_gif(&rgba, output_path)?;
        }
    }

    Ok(())
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
    fs::copy(&entry.source_path, output_path)?;
    Ok(())
}

fn process_animated_gif(entry: &InputEntry, settings: &BatchSettings, output_path: &Path) -> Result<()> {
    let file = File::open(&entry.source_path)?;
    let mut decoder = DecodeOptions::new();
    decoder.set_color_output(ColorOutput::RGBA);
    let mut reader = decoder.read_info(file)?;

    let output = File::create(output_path)?;
    let mut writer = BufWriter::new(output);
    let mut encoder = GifEncoder::new(&mut writer, reader.width(), reader.height(), &[])?;
    encoder.set_repeat(Repeat::Infinite)?;

    while let Some(frame) = reader.read_next_frame()? {
        let rgba = RgbaImage::from_raw(frame.width as u32, frame.height as u32, frame.buffer.to_vec())
            .ok_or_else(|| anyhow!("failed to build GIF frame"))?;
        let resized = resize_dynamic_image(DynamicImage::ImageRgba8(rgba), &settings.resize).to_rgba8();
        let frame_width = resized.width() as u16;
        let frame_height = resized.height() as u16;
        let mut raw = resized.into_raw();
        let mut out_frame = GifFrame::from_rgba_speed(
            frame_width,
            frame_height,
            raw.as_mut_slice(),
            gif_speed_from_colors(settings.quality.gif_colors),
        );
        out_frame.delay = frame.delay;
        out_frame.dispose = frame.dispose;
        out_frame.transparent = frame.transparent;
        encoder.write_frame(&out_frame)?;
    }

    Ok(())
}

fn encode_static_gif(image: &RgbaImage, output_path: &Path) -> Result<()> {
    let output = File::create(output_path)?;
    let mut writer = BufWriter::new(output);
    let mut encoder = GifEncoder::new(&mut writer, image.width() as u16, image.height() as u16, &[])?;
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

fn decode_input_image(entry: &InputEntry) -> Result<DynamicImage> {
    match entry.format {
        InputFormat::Heic | InputFormat::Heif => decode_heif_image(Path::new(&entry.source_path)),
        InputFormat::Avif => decode_avif_image(Path::new(&entry.source_path)),
        _ => {
            let bytes = fs::read(&entry.source_path)?;
            image::load_from_memory_with_format(&bytes, image_format_from_input(&entry.format)?)
                .context("failed to decode image")
        }
    }
}

fn decode_avif_image(path: &Path) -> Result<DynamicImage> {
    let bytes = fs::read(path).with_context(|| format!("failed to read AVIF file: {}", path.display()))?;
    let decoded = zenavif::decode(&bytes).with_context(|| format!("failed to decode AVIF image: {}", path.display()))?;
    dynamic_image_from_zenavif(&decoded)
}

fn decode_heif_image(path: &Path) -> Result<DynamicImage> {
    let decoded = heif_oxide::decode_file(path)
        .with_context(|| format!("failed to decode HEIC / HEIF image: {}", path.display()))?;
    let rgba = decoded.to_rgba8();
    let image = RgbaImage::from_raw(decoded.width, decoded.height, rgba)
        .ok_or_else(|| anyhow!("failed to materialize HEIC / HEIF RGBA buffer"))?;
    Ok(DynamicImage::ImageRgba8(image))
}

fn dynamic_image_from_zenavif(buffer: &zenavif::PixelBuffer) -> Result<DynamicImage> {
    let width = buffer.width() as u32;
    let height = buffer.height() as u32;
    let descriptor = buffer.descriptor();

    let rgba = if descriptor.layout_compatible(PixelDescriptor::RGBA8) {
        let image = buffer
            .try_as_imgref::<rgb::Rgba<u8>>()
            .ok_or_else(|| anyhow!("failed to read AVIF RGBA8 pixels"))?;
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for row in image.rows() {
            for pixel in row {
                out.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
            }
        }
        out
    } else if descriptor.layout_compatible(PixelDescriptor::RGBA16) {
        let image = buffer
            .try_as_imgref::<rgb::Rgba<u16>>()
            .ok_or_else(|| anyhow!("failed to read AVIF RGBA16 pixels"))?;
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for row in image.rows() {
            for pixel in row {
                out.extend_from_slice(&[
                    (pixel.r >> 8) as u8,
                    (pixel.g >> 8) as u8,
                    (pixel.b >> 8) as u8,
                    (pixel.a >> 8) as u8,
                ]);
            }
        }
        out
    } else if descriptor.layout_compatible(PixelDescriptor::RGB8) {
        let image = buffer
            .try_as_imgref::<rgb::Rgb<u8>>()
            .ok_or_else(|| anyhow!("failed to read AVIF RGB8 pixels"))?;
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for row in image.rows() {
            for pixel in row {
                out.extend_from_slice(&[pixel.r, pixel.g, pixel.b, 255]);
            }
        }
        out
    } else if descriptor.layout_compatible(PixelDescriptor::RGB16) {
        let image = buffer
            .try_as_imgref::<rgb::Rgb<u16>>()
            .ok_or_else(|| anyhow!("failed to read AVIF RGB16 pixels"))?;
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for row in image.rows() {
            for pixel in row {
                out.extend_from_slice(&[
                    (pixel.r >> 8) as u8,
                    (pixel.g >> 8) as u8,
                    (pixel.b >> 8) as u8,
                    255,
                ]);
            }
        }
        out
    } else {
        return Err(anyhow!("unsupported AVIF pixel layout"));
    };

    let image = RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| anyhow!("failed to materialize AVIF RGBA buffer"))?;
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
    let Some(value) = settings.value else {
        return image;
    };
    let (width, height) = image.dimensions();
    let (target_width, target_height) = match settings.mode {
        ResizeMode::None => return image,
        ResizeMode::Width => {
            let new_width = value.min(width).max(1);
            let new_height = ((height as f64 * (new_width as f64 / width as f64)).round() as u32).max(1);
            (new_width, new_height)
        }
        ResizeMode::Height => {
            let new_height = value.min(height).max(1);
            let new_width = ((width as f64 * (new_height as f64 / height as f64)).round() as u32).max(1);
            (new_width, new_height)
        }
        ResizeMode::LongEdge => {
            let limited = value.min(width.max(height)).max(1);
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

    if target_width >= width && target_height >= height {
        return image;
    }

    image.resize(target_width, target_height, image::imageops::FilterType::Lanczos3)
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            inspect_inputs,
            process_batch,
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
        };
        let output_root = resolve_output_root(&settings).unwrap();
        let error = process_one(&entry, &settings, &output_root).unwrap_err();
        assert!(error.to_string().contains("アニメーション GIF"));
    }
}
