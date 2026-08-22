//! 動画圧縮モードの処理。
//!
//! エンコードは FFmpeg を外部プロセスとして駆動する（`docs/decision-log.md` の `D-18`）。
//! ライブラリとしてリンクしないため、クラッシュがアプリ本体へ波及せず、
//! `-progress pipe:1` で進捗が取れ、stdin へ `q` を送って正常終了させられる。

use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{Emitter, Window};
use walkdir::WalkDir;

use crate::{
    apply_timestamps_from, build_output_path_from, catch_task_panic, default_output_root,
    is_hidden_or_system, BatchControl, MetadataMode, OutputMode, ResizeMode, ResizeSettings,
    ResizeUnit, SkippedItem, TimestampSettings,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// GUI から子プロセスを起こすときにコンソールを出さない。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 入力として受け付ける拡張子。実際の判定は ffprobe が行う。
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "webm", "mkv", "avi", "wmv", "mts", "m2ts",
];

/// コピーのまま MP4 に入れられる音声コデック。
const MP4_AUDIO_PASSTHROUGH: &[&str] = &["aac", "mp3", "alac"];

/// 長尺警告のしきい値。
const LONG_DURATION_SEC: f64 = 20.0 * 60.0;

// ---------------------------------------------------------------------------
// 設定
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VideoOutputFormat {
    /// MP4 (H.264 + AAC)。Phase 1 の唯一の出力。
    Mp4H264,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum QualityPreset {
    High,
    Standard,
    Small,
    Smallest,
}

impl QualityPreset {
    /// CRF 対応エンコーダ向けの値。数字が小さいほど高品質。
    fn crf(self) -> u32 {
        match self {
            QualityPreset::High => 18,
            QualityPreset::Standard => 23,
            QualityPreset::Small => 28,
            QualityPreset::Smallest => 32,
        }
    }

    /// ビットレート指定しかできないエンコーダ向けの bits per pixel。
    fn bits_per_pixel(self) -> f64 {
        match self {
            QualityPreset::High => 0.12,
            QualityPreset::Standard => 0.08,
            QualityPreset::Small => 0.05,
            QualityPreset::Smallest => 0.03,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AudioMode {
    Copy,
    Aac,
    Remove,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoSettings {
    /// Phase 1 では MP4 (H.264) のみ。UI との契約を保つために受け取る。
    #[allow(dead_code)]
    output_format: VideoOutputFormat,
    output_mode: OutputMode,
    custom_output_dir: Option<String>,
    overwrite: bool,
    resize: ResizeSettings,
    quality_preset: QualityPreset,
    /// 詳細指定。CRF 対応エンコーダのときだけ効く。
    crf_override: Option<u32>,
    fps_limit: Option<u32>,
    audio_mode: AudioMode,
    audio_bitrate_kbps: u32,
    metadata_mode: MetadataMode,
    timestamps: TimestampSettings,
    /// 同梱バイナリより優先して使う ffmpeg のパス。
    ffmpeg_path: Option<String>,
}

// ---------------------------------------------------------------------------
// 入力
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoInputEntry {
    id: String,
    source_path: String,
    root_path: String,
    relative_path: String,
    file_name: String,
    /// 表の「形式」列に出す文字列。
    format_label: String,
    video_codec: String,
    audio_codec: Option<String>,
    file_size: u64,
    width: Option<u32>,
    height: Option<u32>,
    duration_sec: Option<f64>,
    fps: Option<f64>,
    variable_frame_rate: bool,
    bit_rate: Option<u64>,
    rotation: Option<i32>,
    has_audio: bool,
    audio_track_count: usize,
    subtitle_track_count: usize,
    hdr: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoInspectResponse {
    entries: Vec<VideoInputEntry>,
    skipped: Vec<SkippedItem>,
    /// 対象外種別の集約件数。1 件 1 行にすると一覧が埋まるため分けている。
    excluded_count: usize,
}

// ---------------------------------------------------------------------------
// 結果と進捗
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoResultItem {
    source_path: String,
    output_path: Option<String>,
    success: bool,
    /// 停止操作で打ち切ったもの。失敗とは区別して表示する。
    interrupted: bool,
    output_format: Option<String>,
    original_size: u64,
    optimized_size: Option<u64>,
    saved_size: Option<i64>,
    saved_percent: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
    duration_sec: Option<f64>,
    reason: Option<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoProcessResponse {
    results: Vec<VideoResultItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoProcessRequest {
    entries: Vec<VideoInputEntry>,
    settings: VideoSettings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VideoProgress {
    completed: usize,
    total: usize,
    current_path: Option<String>,
    state: &'static str,
    /// 現在処理中のファイル内の進捗率 (0-100)。動画は 1 件が長いため必要。
    current_file_percent: Option<f64>,
}

// ---------------------------------------------------------------------------
// FFmpeg の解決
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct FfmpegTools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    /// "setting" / "bundled" / "path"
    source: &'static str,
    version: String,
    encoders: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RateControl {
    /// 固定品質。CRF を指定できるエンコーダ。
    Crf,
    /// ビットレート指定のみ。OS / ハードウェアエンコーダの多くがこちら。
    Bitrate,
}

impl RateControl {
    fn as_str(self) -> &'static str {
        match self {
            RateControl::Crf => "crf",
            RateControl::Bitrate => "bitrate",
        }
    }
}

/// H.264 エンコーダの優先順。
///
/// `D-18` により同梱ビルドは LGPL 構成で `libx264` を含まないため、通常は
/// OS / ハードウェアエンコーダが選ばれる。ユーザーが外部の GPL ビルドを
/// 指定した場合はそちらの `libx264` を使う。
const H264_CANDIDATES: &[(&str, RateControl)] = &[
    ("libx264", RateControl::Crf),
    ("h264_nvenc", RateControl::Bitrate),
    ("h264_qsv", RateControl::Bitrate),
    ("h264_amf", RateControl::Bitrate),
    ("h264_videotoolbox", RateControl::Bitrate),
    ("h264_mf", RateControl::Bitrate),
    ("libopenh264", RateControl::Bitrate),
];

fn pick_h264(encoders: &HashSet<String>) -> Option<(&'static str, RateControl)> {
    H264_CANDIDATES
        .iter()
        .find(|(name, _)| encoders.contains(*name))
        .map(|(name, rate)| (*name, *rate))
}

fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn sibling_ffprobe(ffmpeg: &Path) -> PathBuf {
    let name = if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" };
    match ffmpeg.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

/// 同梱バイナリの置き場所。exe と同じ階層と `binaries/` を見る。
fn bundled_candidates() -> Vec<PathBuf> {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(name));
            candidates.push(dir.join("binaries").join(name));
        }
    }
    candidates
}

fn probe_version(ffmpeg: &Path) -> Result<String> {
    let output = command(ffmpeg)
        .args(["-hide_banner", "-version"])
        .output()
        .with_context(|| format!("failed to run {}", ffmpeg.display()))?;
    if !output.status.success() {
        return Err(anyhow!("{} -version failed", ffmpeg.display()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.lines().next().unwrap_or("ffmpeg").trim().to_string())
}

fn list_encoders(ffmpeg: &Path) -> HashSet<String> {
    let Ok(output) = command(ffmpeg).args(["-hide_banner", "-encoders"]).output() else {
        return HashSet::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut encoders = HashSet::new();
    for line in text.lines() {
        // ` V..... libx264              libx264 H.264 ...` の 2 列目を拾う。
        let trimmed = line.trim_start();
        let mut parts = trimmed.split_whitespace();
        let flags = parts.next().unwrap_or("");
        if flags.len() < 6 || !(flags.starts_with('V') || flags.starts_with('A')) {
            continue;
        }
        if let Some(name) = parts.next() {
            encoders.insert(name.to_string());
        }
    }
    encoders
}

fn resolve_tools(explicit: Option<&str>) -> Result<FfmpegTools> {
    let mut attempts: Vec<(PathBuf, &'static str)> = Vec::new();

    if let Some(raw) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(raw);
        // ディレクトリを指定された場合は中の ffmpeg を見る。
        if path.is_dir() {
            let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
            attempts.push((path.join(name), "setting"));
        } else {
            attempts.push((path, "setting"));
        }
    }
    for candidate in bundled_candidates() {
        attempts.push((candidate, "bundled"));
    }
    attempts.push((PathBuf::from("ffmpeg"), "path"));

    let mut last_error = None;
    for (candidate, source) in attempts {
        // 明示指定と PATH 以外は、存在しないものを試さない。
        if source == "bundled" && !candidate.exists() {
            continue;
        }
        match probe_version(&candidate) {
            Ok(version) => {
                let ffprobe = sibling_ffprobe(&candidate);
                let ffprobe = if source == "path" || !ffprobe.exists() {
                    // PATH 経由なら ffprobe も PATH から引く。
                    if ffprobe.exists() {
                        ffprobe
                    } else {
                        PathBuf::from(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" })
                    }
                } else {
                    ffprobe
                };
                let encoders = list_encoders(&candidate);
                return Ok(FfmpegTools {
                    ffmpeg: candidate,
                    ffprobe,
                    source,
                    version,
                    encoders,
                });
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(anyhow!(
        "FFmpeg が見つかりません。設定で ffmpeg のパスを指定するか、PATH に追加してください。{}",
        last_error
            .map(|error| format!(" (最後のエラー: {error})"))
            .unwrap_or_default()
    ))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoEnvironment {
    available: bool,
    ffmpeg_path: Option<String>,
    ffprobe_path: Option<String>,
    version: Option<String>,
    source: Option<String>,
    video_encoder: Option<String>,
    rate_control: Option<String>,
    message: Option<String>,
}

#[tauri::command]
pub(crate) async fn video_environment(ffmpeg_path: Option<String>) -> Result<VideoEnvironment, String> {
    tauri::async_runtime::spawn_blocking(move || match resolve_tools(ffmpeg_path.as_deref()) {
        Ok(tools) => {
            let picked = pick_h264(&tools.encoders);
            VideoEnvironment {
                available: picked.is_some(),
                ffmpeg_path: Some(tools.ffmpeg.to_string_lossy().to_string()),
                ffprobe_path: Some(tools.ffprobe.to_string_lossy().to_string()),
                version: Some(tools.version),
                source: Some(tools.source.to_string()),
                video_encoder: picked.map(|(name, _)| name.to_string()),
                rate_control: picked.map(|(_, rate)| rate.as_str().to_string()),
                message: match picked {
                    Some(_) => None,
                    None => Some(
                        "この FFmpeg には利用できる H.264 エンコーダがありません。".to_string(),
                    ),
                },
            }
        }
        Err(error) => VideoEnvironment {
            available: false,
            ffmpeg_path: None,
            ffprobe_path: None,
            version: None,
            source: None,
            video_encoder: None,
            rate_control: None,
            message: Some(format!("{error:#}")),
        },
    })
    .await
    .map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// 入力の読込
// ---------------------------------------------------------------------------

fn has_video_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| VIDEO_EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[tauri::command]
pub(crate) async fn inspect_video_inputs(
    paths: Vec<String>,
    ffmpeg_path: Option<String>,
) -> Result<VideoInspectResponse, String> {
    tauri::async_runtime::spawn_blocking(move || inspect_impl(paths, ffmpeg_path.as_deref()))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))
}

fn inspect_impl(paths: Vec<String>, ffmpeg_path: Option<&str>) -> Result<VideoInspectResponse> {
    let tools = resolve_tools(ffmpeg_path)?;

    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut excluded_count = 0usize;
    let mut seen = HashSet::new();

    for raw in paths {
        let path = PathBuf::from(&raw);
        if path.is_dir() {
            for entry in WalkDir::new(&path).into_iter().filter_map(Result::ok) {
                let candidate = entry.path();
                if candidate.is_dir() || is_hidden_or_system(candidate) {
                    continue;
                }
                if !has_video_extension(candidate) {
                    excluded_count += 1;
                    continue;
                }
                let relative = candidate
                    .strip_prefix(&path)
                    .unwrap_or(candidate)
                    .to_string_lossy()
                    .to_string();
                push_entry(&tools, candidate, &path, &relative, &mut entries, &mut skipped, &mut seen);
            }
            continue;
        }

        if !path.is_file() {
            skipped.push(SkippedItem {
                path: raw.clone(),
                reason: "ファイルが見つかりません。".to_string(),
            });
            continue;
        }
        if !has_video_extension(&path) {
            excluded_count += 1;
            continue;
        }
        let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let relative = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| raw.clone());
        push_entry(&tools, &path, &root, &relative, &mut entries, &mut skipped, &mut seen);
    }

    entries.sort_by(|left, right| left.source_path.cmp(&right.source_path));

    Ok(VideoInspectResponse {
        entries,
        skipped,
        excluded_count,
    })
}

fn push_entry(
    tools: &FfmpegTools,
    path: &Path,
    root: &Path,
    relative: &str,
    entries: &mut Vec<VideoInputEntry>,
    skipped: &mut Vec<SkippedItem>,
    seen: &mut HashSet<String>,
) {
    let key = path.to_string_lossy().to_string();
    if !seen.insert(key) {
        return;
    }
    match inspect_one(tools, path, root, relative) {
        Ok(entry) => entries.push(entry),
        Err(error) => skipped.push(SkippedItem {
            path: path.to_string_lossy().to_string(),
            reason: format!("{error:#}"),
        }),
    }
}

fn ffprobe_json(tools: &FfmpegTools, path: &Path) -> Result<Value> {
    let output = command(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .with_context(|| format!("failed to run {}", tools.ffprobe.display()))?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(if message.is_empty() {
            "ffprobe で情報を取得できませんでした。".to_string()
        } else {
            message
        }));
    }

    serde_json::from_slice(&output.stdout).context("ffprobe の出力を解釈できませんでした")
}

/// `30000/1001` 形式の分数を fps に直す。
fn parse_rational(value: Option<&str>) -> Option<f64> {
    let raw = value?;
    let mut parts = raw.split('/');
    let numerator: f64 = parts.next()?.parse().ok()?;
    let denominator: f64 = parts.next().unwrap_or("1").parse().ok()?;
    if denominator == 0.0 {
        return None;
    }
    let fps = numerator / denominator;
    if fps <= 0.0 {
        None
    } else {
        Some(fps)
    }
}

fn stream_rotation(stream: &Value) -> Option<i32> {
    if let Some(list) = stream.get("side_data_list").and_then(Value::as_array) {
        for item in list {
            if let Some(rotation) = item.get("rotation").and_then(Value::as_f64) {
                return Some(rotation.round() as i32);
            }
        }
    }
    stream
        .get("tags")
        .and_then(|tags| tags.get("rotate"))
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<i32>().ok())
}

fn inspect_one(
    tools: &FfmpegTools,
    path: &Path,
    root: &Path,
    relative: &str,
) -> Result<VideoInputEntry> {
    let probe = ffprobe_json(tools, path)?;
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let video = streams
        .iter()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("video"))
        .ok_or_else(|| anyhow!("映像ストリームが見つかりません。"))?;

    let audio_streams: Vec<&Value> = streams
        .iter()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"))
        .collect();
    let subtitle_count = streams
        .iter()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("subtitle"))
        .count();

    let metadata = fs::metadata(path)?;
    let width = video.get("width").and_then(Value::as_u64).map(|v| v as u32);
    let height = video.get("height").and_then(Value::as_u64).map(|v| v as u32);
    let video_codec = video
        .get("codec_name")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let audio_codec = audio_streams
        .first()
        .and_then(|stream| stream.get("codec_name"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let avg_fps = parse_rational(video.get("avg_frame_rate").and_then(Value::as_str));
    let real_fps = parse_rational(video.get("r_frame_rate").and_then(Value::as_str));
    // 厳密な判定にはフレーム単位の解析が必要なため、2 つのレートの差で近似する。
    let variable_frame_rate = match (avg_fps, real_fps) {
        (Some(avg), Some(real)) => (avg - real).abs() > 0.5,
        _ => false,
    };

    let duration_sec = probe
        .get("format")
        .and_then(|format| format.get("duration"))
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| *value > 0.0);

    let bit_rate = probe
        .get("format")
        .and_then(|format| format.get("bit_rate"))
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u64>().ok());

    let transfer = video
        .get("color_transfer")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let primaries = video
        .get("color_primaries")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let hdr = matches!(transfer.as_str(), "smpte2084" | "arib-std-b67") || primaries == "bt2020";

    let container = probe
        .get("format")
        .and_then(|format| format.get("format_name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .split(',')
        .next()
        .unwrap_or("unknown")
        .to_string();

    let mut warnings = Vec::new();
    if let Some(duration) = duration_sec {
        if duration > LONG_DURATION_SEC {
            warnings.push(format!("長尺 ({:.0} 分)", duration / 60.0));
        }
    } else {
        warnings.push("再生時間が取得できません".to_string());
    }
    if let (Some(w), Some(h)) = (width, height) {
        if w.max(h) > 2560 {
            warnings.push(format!("高解像度 {w}x{h}"));
        }
    }
    if hdr {
        warnings.push("HDR (トーンマッピングは行いません)".to_string());
    }
    if variable_frame_rate {
        warnings.push("可変フレームレート (固定化します)".to_string());
    }
    if audio_streams.len() > 1 {
        warnings.push(format!("副音声 {} 本は破棄されます", audio_streams.len() - 1));
    }
    if subtitle_count > 0 {
        warnings.push(format!("字幕 {subtitle_count} 本は破棄されます"));
    }

    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| relative.to_string());

    Ok(VideoInputEntry {
        id: path.to_string_lossy().to_string(),
        source_path: path.to_string_lossy().to_string(),
        root_path: root.to_string_lossy().to_string(),
        relative_path: relative.to_string(),
        file_name,
        format_label: format!("{} / {}", container.to_uppercase(), video_codec),
        video_codec,
        audio_codec,
        file_size: metadata.len(),
        width,
        height,
        duration_sec,
        fps: avg_fps.or(real_fps),
        variable_frame_rate,
        bit_rate,
        rotation: stream_rotation(video),
        has_audio: !audio_streams.is_empty(),
        audio_track_count: audio_streams.len(),
        subtitle_track_count: subtitle_count,
        hdr,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// エンコード
// ---------------------------------------------------------------------------

/// リサイズのフィルタ式。
///
/// `iw` / `ih` は自動回転後の寸法になるため、縦向き動画でも意図どおりに効く。
/// `trunc(x/2)*2` と `-2` で yuv420 が要求する偶数寸法を満たす。
fn scale_filter(resize: &ResizeSettings) -> String {
    let Some(value) = resize.value.filter(|value| *value > 0) else {
        return "scale=w='trunc(iw/2)*2':h='trunc(ih/2)*2'".to_string();
    };

    match (&resize.mode, &resize.unit) {
        (ResizeMode::None, _) => "scale=w='trunc(iw/2)*2':h='trunc(ih/2)*2'".to_string(),
        // % 指定は基準に関係なく一様縮小になる。
        (_, ResizeUnit::Percent) => {
            let percent = value.min(100);
            format!("scale=w='trunc(iw*{percent}/200)*2':h=-2")
        }
        (ResizeMode::Width, ResizeUnit::Px) => {
            format!("scale=w='trunc(min(iw,{value})/2)*2':h=-2")
        }
        (ResizeMode::Height, ResizeUnit::Px) => {
            format!("scale=w=-2:h='trunc(min(ih,{value})/2)*2'")
        }
        (ResizeMode::LongEdge, ResizeUnit::Px) => format!(
            "scale=w='if(gte(iw,ih),trunc(min(iw,{value})/2)*2,-2)':h='if(gte(iw,ih),-2,trunc(min(ih,{value})/2)*2)'"
        ),
    }
}

fn video_filter(entry: &VideoInputEntry, settings: &VideoSettings) -> String {
    let mut parts = Vec::new();

    // fps は下げるときだけ入れる。上げるとフレーム複製で無駄に太る。
    match (settings.fps_limit, entry.fps) {
        (Some(limit), Some(source)) if source > f64::from(limit) + 0.01 => {
            parts.push(format!("fps={limit}"));
        }
        _ => {
            if entry.variable_frame_rate {
                if let Some(source) = entry.fps {
                    parts.push(format!("fps={:.3}", source));
                }
            }
        }
    }

    parts.push(scale_filter(&settings.resize));
    parts.join(",")
}

/// 出力寸法の見積り。ビットレート算出にのみ使う。
///
/// 実際の寸法は ffmpeg がフィルタ式から決めるため、ここでは回転を考慮した
/// 表示寸法から概算する。
fn estimated_output_dimensions(entry: &VideoInputEntry, settings: &VideoSettings) -> (u32, u32) {
    let (mut width, mut height) = (entry.width.unwrap_or(1920), entry.height.unwrap_or(1080));
    if matches!(entry.rotation, Some(90) | Some(-90) | Some(270) | Some(-270)) {
        std::mem::swap(&mut width, &mut height);
    }

    let Some(value) = settings.resize.value.filter(|value| *value > 0) else {
        return (width, height);
    };

    let scale = match (&settings.resize.mode, &settings.resize.unit) {
        (ResizeMode::None, _) => 1.0,
        (_, ResizeUnit::Percent) => f64::from(value.min(100)) / 100.0,
        (ResizeMode::Width, ResizeUnit::Px) => f64::from(value).min(f64::from(width)) / f64::from(width),
        (ResizeMode::Height, ResizeUnit::Px) => {
            f64::from(value).min(f64::from(height)) / f64::from(height)
        }
        (ResizeMode::LongEdge, ResizeUnit::Px) => {
            let long = f64::from(width.max(height));
            f64::from(value).min(long) / long
        }
    };

    let scaled_width = ((f64::from(width) * scale).round() as u32).max(2);
    let scaled_height = ((f64::from(height) * scale).round() as u32).max(2);
    (scaled_width & !1, scaled_height & !1)
}

fn target_bitrate_kbps(entry: &VideoInputEntry, settings: &VideoSettings) -> u32 {
    let (width, height) = estimated_output_dimensions(entry, settings);
    let fps = match (settings.fps_limit, entry.fps) {
        (Some(limit), Some(source)) => source.min(f64::from(limit)),
        (Some(limit), None) => f64::from(limit),
        (None, Some(source)) => source,
        (None, None) => 30.0,
    };
    let bits = f64::from(width) * f64::from(height) * fps * settings.quality_preset.bits_per_pixel();
    let kbps = (bits / 1000.0).round() as u32;
    kbps.clamp(200, 100_000)
}

struct EncodePlan {
    args: Vec<String>,
    warnings: Vec<String>,
    encoder: &'static str,
}

fn build_encode_plan(
    entry: &VideoInputEntry,
    settings: &VideoSettings,
    tools: &FfmpegTools,
    output: &Path,
) -> Result<EncodePlan> {
    let (encoder, rate) = pick_h264(&tools.encoders).ok_or_else(|| {
        anyhow!("この FFmpeg には利用できる H.264 エンコーダがありません。")
    })?;

    let mut warnings = Vec::new();
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostats".into(),
        "-y".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-i".into(),
        entry.source_path.clone(),
        "-map".into(),
        "0:v:0".into(),
    ];

    let remove_audio = matches!(settings.audio_mode, AudioMode::Remove) || !entry.has_audio;
    if !remove_audio {
        args.push("-map".into());
        args.push("0:a:0?".into());
    }

    args.push("-vf".into());
    args.push(video_filter(entry, settings));

    args.push("-c:v".into());
    args.push(encoder.to_string());

    match rate {
        RateControl::Crf => {
            let crf = settings.crf_override.unwrap_or_else(|| settings.quality_preset.crf());
            args.push("-crf".into());
            args.push(crf.to_string());
            args.push("-preset".into());
            args.push("medium".into());
        }
        RateControl::Bitrate => {
            if settings.crf_override.is_some() {
                warnings.push(format!(
                    "{encoder} は CRF 指定に対応しないため、品質プリセットからビットレートを算出しました"
                ));
            }
            let kbps = target_bitrate_kbps(entry, settings);
            args.push("-b:v".into());
            args.push(format!("{kbps}k"));
            args.push("-maxrate".into());
            args.push(format!("{}k", kbps * 3 / 2));
            args.push("-bufsize".into());
            args.push(format!("{}k", kbps * 3));
        }
    }

    args.push("-pix_fmt".into());
    args.push("yuv420p".into());

    if remove_audio {
        args.push("-an".into());
    } else {
        let passthrough = entry
            .audio_codec
            .as_deref()
            .map(|codec| MP4_AUDIO_PASSTHROUGH.contains(&codec))
            .unwrap_or(false);
        match settings.audio_mode {
            AudioMode::Copy if passthrough => {
                args.push("-c:a".into());
                args.push("copy".into());
            }
            AudioMode::Copy => {
                warnings.push(format!(
                    "{} は MP4 へそのまま入れられないため AAC へ変換しました",
                    entry.audio_codec.as_deref().unwrap_or("この音声")
                ));
                args.push("-c:a".into());
                args.push("aac".into());
                args.push("-b:a".into());
                args.push(format!("{}k", settings.audio_bitrate_kbps));
            }
            AudioMode::Aac => {
                args.push("-c:a".into());
                args.push("aac".into());
                args.push("-b:a".into());
                args.push(format!("{}k", settings.audio_bitrate_kbps));
            }
            AudioMode::Remove => unreachable!("remove_audio で分岐済み"),
        }
    }

    if matches!(settings.metadata_mode, MetadataMode::Strip) {
        // 回転はフィルタ側で焼き込まれるため、メタデータを落としても向きは保たれる。
        args.push("-map_metadata".into());
        args.push("-1".into());
    }

    if entry.hdr {
        warnings.push("HDR 入力を SDR へ変換せず出力するため、色が変わる場合があります".to_string());
    }

    args.push("-movflags".into());
    args.push("+faststart".into());
    args.push("-f".into());
    args.push("mp4".into());
    args.push(output.to_string_lossy().to_string());

    Ok(EncodePlan {
        args,
        warnings,
        encoder,
    })
}

enum RunOutcome {
    Completed,
    Interrupted,
}

/// ffmpeg を起動し、進捗を流しながら完了を待つ。
///
/// 停止要求が来たら stdin へ `q` を送る。強制終了ではコンテナが閉じられず、
/// ファイルハンドルも残るため、まず正常終了を試みる（`D-20`）。
fn run_ffmpeg(
    window: &Window,
    control: &BatchControl,
    tools: &FfmpegTools,
    plan: &EncodePlan,
    entry: &VideoInputEntry,
    index: usize,
    total: usize,
) -> Result<RunOutcome> {
    let mut child = command(&tools.ffmpeg)
        .args(&plan.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {}", tools.ffmpeg.display()))?;

    let mut stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("ffmpeg の stdout を取得できませんでした"))?;
    let mut stderr = child.stderr.take();

    // stderr を読まないとパイプが詰まって ffmpeg が止まるため、別スレッドで捨てずに溜める。
    let stderr_handle = std::thread::spawn(move || {
        let mut buffer = String::new();
        if let Some(stream) = stderr.as_mut() {
            let _ = stream.read_to_string(&mut buffer);
        }
        buffer
    });

    let started = Instant::now();
    // 再生時間の 20 倍を超えたら異常とみなす。取得できない場合は 1 時間。
    let timeout = entry
        .duration_sec
        .map(|duration| Duration::from_secs_f64((duration * 20.0).max(120.0)))
        .unwrap_or_else(|| Duration::from_secs(3600));

    let mut interrupted = false;
    let mut last_emitted = -1.0f64;
    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };

        if let Some(raw) = line.strip_prefix("out_time_us=") {
            if let (Ok(micros), Some(duration)) = (raw.trim().parse::<f64>(), entry.duration_sec) {
                let percent = ((micros / 1_000_000.0) / duration * 100.0).clamp(0.0, 100.0);
                if percent - last_emitted >= 0.5 {
                    last_emitted = percent;
                    emit_progress(
                        window,
                        index,
                        total,
                        Some(entry.source_path.clone()),
                        "running",
                        Some(percent),
                    );
                }
            }
        }

        if control.stop_requested.load(Ordering::SeqCst) {
            interrupted = true;
            if let Some(pipe) = stdin.as_mut() {
                let _ = pipe.write_all(b"q\n");
                let _ = pipe.flush();
            }
            break;
        }

        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "処理時間が想定を大きく超えたため中断しました（{} 秒経過）",
                started.elapsed().as_secs()
            ));
        }
    }

    if interrupted {
        // 正常終了を待つ。閉じきらない場合だけ強制終了する。
        let grace = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if grace.elapsed() > Duration::from_secs(10) {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }
        drop(stdin);
        let _ = stderr_handle.join();
        return Ok(RunOutcome::Interrupted);
    }

    drop(stdin);
    let status = child.wait()?;
    let stderr_text = stderr_handle.join().unwrap_or_default();

    if !status.success() {
        let tail: Vec<&str> = stderr_text.lines().rev().take(6).collect();
        let detail = tail.into_iter().rev().collect::<Vec<_>>().join(" / ");
        return Err(anyhow!(
            "ffmpeg が失敗しました (exit {}){}",
            status.code().unwrap_or(-1),
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }

    Ok(RunOutcome::Completed)
}

fn emit_progress(
    window: &Window,
    completed: usize,
    total: usize,
    current_path: Option<String>,
    state: &'static str,
    current_file_percent: Option<f64>,
) {
    let _ = window.emit(
        "batch-progress",
        VideoProgress {
            completed,
            total,
            current_path,
            state,
            current_file_percent,
        },
    );
}

fn wait_until_can_continue(
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
        emit_progress(window, completed, total, current_path.clone(), "paused", None);
        std::thread::sleep(Duration::from_millis(120));
    }

    if control.stop_requested.load(Ordering::SeqCst) {
        emit_progress(window, completed, total, current_path, "stopping", None);
        return false;
    }

    true
}

fn resolve_output_root(settings: &VideoSettings) -> Result<PathBuf> {
    match settings.output_mode {
        OutputMode::DesktopDefault => default_output_root(),
        OutputMode::Custom => match settings.custom_output_dir.as_deref() {
            Some(path) if !path.trim().is_empty() => Ok(PathBuf::from(path)),
            _ => default_output_root(),
        },
    }
}

/// 形式・寸法・フレームレート・音声のいずれも変えない指定かどうか。
///
/// これが真のときに出力が元より大きくなった場合は、元ファイルをコピーする。
fn is_passthrough_request(entry: &VideoInputEntry, settings: &VideoSettings) -> bool {
    matches!(settings.resize.mode, ResizeMode::None)
        && settings.fps_limit.is_none()
        && matches!(settings.audio_mode, AudioMode::Copy)
        && entry.video_codec == "h264"
}

#[tauri::command]
pub(crate) async fn process_video_batch(
    window: Window,
    control: tauri::State<'_, BatchControl>,
    request: VideoProcessRequest,
) -> Result<VideoProcessResponse, String> {
    control.paused.store(false, Ordering::SeqCst);
    control.stop_requested.store(false, Ordering::SeqCst);
    let control = control.inner().clone();

    tauri::async_runtime::spawn_blocking(move || process_impl(window, request, control))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))
}

fn process_impl(
    window: Window,
    request: VideoProcessRequest,
    control: BatchControl,
) -> Result<VideoProcessResponse> {
    let tools = resolve_tools(request.settings.ffmpeg_path.as_deref())?;
    let output_root = resolve_output_root(&request.settings)?;
    fs::create_dir_all(&output_root)
        .with_context(|| format!("failed to create output root: {}", output_root.display()))?;

    let total = request.entries.len();
    let mut results = Vec::with_capacity(total);

    for (index, entry) in request.entries.iter().enumerate() {
        if !wait_until_can_continue(
            &window,
            &control,
            index,
            total,
            Some(entry.source_path.clone()),
        ) {
            break;
        }

        emit_progress(
            &window,
            index,
            total,
            Some(entry.source_path.clone()),
            "running",
            Some(0.0),
        );

        let result = catch_task_panic("video processing", || {
            process_one(
                &window,
                &control,
                &tools,
                entry,
                &request.settings,
                &output_root,
                index,
                total,
            )
        })
        .unwrap_or_else(|error| failure_item(entry, format!("{error:#}")));

        let interrupted = result.interrupted;
        results.push(result);

        emit_progress(
            &window,
            index + 1,
            total,
            Some(entry.source_path.clone()),
            "running",
            None,
        );

        if interrupted || control.stop_requested.load(Ordering::SeqCst) {
            emit_progress(
                &window,
                index + 1,
                total,
                Some(entry.source_path.clone()),
                "stopping",
                None,
            );
            break;
        }
    }

    Ok(VideoProcessResponse { results })
}

fn failure_item(entry: &VideoInputEntry, reason: String) -> VideoResultItem {
    VideoResultItem {
        source_path: entry.source_path.clone(),
        output_path: None,
        success: false,
        interrupted: false,
        output_format: None,
        original_size: entry.file_size,
        optimized_size: None,
        saved_size: None,
        saved_percent: None,
        width: entry.width,
        height: entry.height,
        duration_sec: entry.duration_sec,
        reason: Some(reason),
        warnings: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn process_one(
    window: &Window,
    control: &BatchControl,
    tools: &FfmpegTools,
    entry: &VideoInputEntry,
    settings: &VideoSettings,
    output_root: &Path,
    index: usize,
    total: usize,
) -> Result<VideoResultItem> {
    let output_path = build_output_path_from(
        &entry.relative_path,
        output_root,
        "mp4",
        settings.overwrite,
    )?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory: {}", parent.display()))?;
    }

    // 出力は一時ファイルへ書き、成功時にリネームする。
    // 途中のファイルを完成品として残さないため（`D-20`）。
    let temp_path = output_path.with_extension("mp4.part");
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }

    let plan = build_encode_plan(entry, settings, tools, &temp_path)?;
    let outcome = run_ffmpeg(window, control, tools, &plan, entry, index, total);

    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
    };

    if matches!(outcome, RunOutcome::Interrupted) {
        let _ = fs::remove_file(&temp_path);
        return Ok(VideoResultItem {
            source_path: entry.source_path.clone(),
            output_path: None,
            success: false,
            interrupted: true,
            output_format: None,
            original_size: entry.file_size,
            optimized_size: None,
            saved_size: None,
            saved_percent: None,
            width: entry.width,
            height: entry.height,
            duration_sec: entry.duration_sec,
            reason: Some("中断（出力を破棄しました）".to_string()),
            warnings: Vec::new(),
        });
    }

    let mut warnings = plan.warnings.clone();
    let encoded_size = fs::metadata(&temp_path)
        .with_context(|| format!("出力が見つかりません: {}", temp_path.display()))?
        .len();

    // 何も変えない指定で太った場合は元をコピーする。画像モードと同じ扱い。
    let use_source_copy = encoded_size > entry.file_size && is_passthrough_request(entry, settings);
    if use_source_copy {
        let _ = fs::remove_file(&temp_path);
        fs::copy(&entry.source_path, &output_path).with_context(|| {
            format!("failed to copy source to {}", output_path.display())
        })?;
        warnings.push("再圧縮すると大きくなるため、元のファイルをコピーしました".to_string());
    } else {
        fs::rename(&temp_path, &output_path).with_context(|| {
            format!("failed to finalize output: {}", output_path.display())
        })?;
    }

    let final_size = fs::metadata(&output_path)?.len();
    let _ = apply_timestamps_from(
        Path::new(&entry.source_path),
        &settings.timestamps,
        &output_path,
    );

    let (width, height) = output_dimensions(tools, &output_path).unwrap_or((entry.width, entry.height));
    let saved = entry.file_size as i64 - final_size as i64;
    let saved_percent = if entry.file_size > 0 {
        Some(saved as f64 / entry.file_size as f64 * 100.0)
    } else {
        None
    };

    Ok(VideoResultItem {
        source_path: entry.source_path.clone(),
        output_path: Some(output_path.to_string_lossy().to_string()),
        success: true,
        interrupted: false,
        output_format: Some(if use_source_copy {
            "copy".to_string()
        } else {
            format!("mp4 / {}", plan.encoder)
        }),
        original_size: entry.file_size,
        optimized_size: Some(final_size),
        saved_size: Some(saved),
        saved_percent,
        width,
        height,
        duration_sec: entry.duration_sec,
        reason: None,
        warnings,
    })
}

fn output_dimensions(tools: &FfmpegTools, path: &Path) -> Option<(Option<u32>, Option<u32>)> {
    let probe = ffprobe_json(tools, path).ok()?;
    let stream = probe
        .get("streams")
        .and_then(Value::as_array)?
        .iter()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("video"))?
        .clone();
    Some((
        stream.get("width").and_then(Value::as_u64).map(|v| v as u32),
        stream.get("height").and_then(Value::as_u64).map(|v| v as u32),
    ))
}
