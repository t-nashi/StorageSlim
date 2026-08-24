//! 動画圧縮モードの処理。
//!
//! エンコードは FFmpeg を外部プロセスとして駆動する（`docs/decision-log.md` の `D-18`）。
//! ライブラリとしてリンクしないため、クラッシュがアプリ本体へ波及せず、
//! `-progress pipe:1` で進捗が取れ、stdin へ `q` を送って正常終了させられる。

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{atomic::Ordering, Mutex, OnceLock},
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

/// 長尺警告のしきい値。
const LONG_DURATION_SEC: f64 = 20.0 * 60.0;

// ---------------------------------------------------------------------------
// 設定
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VideoOutputFormat {
    /// MP4 (H.264 + AAC)。互換性を最優先する既定の出力。
    Mp4H264,
    /// WebM (VP9 + Opus)。同じ体感画質で MP4 より小さくなるが、エンコードは遅い。
    WebmVp9,
}

impl VideoOutputFormat {
    fn extension(self) -> &'static str {
        match self {
            VideoOutputFormat::Mp4H264 => "mp4",
            VideoOutputFormat::WebmVp9 => "webm",
        }
    }

    /// `-f` に渡す muxer 名。一時ファイルが `*.part` で拡張子から推測できないため明示する。
    fn muxer(self) -> &'static str {
        match self {
            VideoOutputFormat::Mp4H264 => "mp4",
            VideoOutputFormat::WebmVp9 => "webm",
        }
    }

    /// 再エンコードせずコピーできる入力コデック。元ファイルコピーの判定に使う。
    fn video_codec_name(self) -> &'static str {
        match self {
            VideoOutputFormat::Mp4H264 => "h264",
            VideoOutputFormat::WebmVp9 => "vp9",
        }
    }

    /// 映像エンコーダの優先順。
    ///
    /// `D-18` により同梱ビルドは LGPL 構成で `libx264` を含まないため、H.264 では
    /// 通常 OS / ハードウェアエンコーダが選ばれる。VP9 の `libvpx` は BSD なので
    /// 同梱ビルドでもソフトウェアエンコーダが使える。
    fn candidates(self) -> &'static [(&'static str, RateControl)] {
        match self {
            VideoOutputFormat::Mp4H264 => &[
                ("libx264", RateControl::Crf),
                ("h264_nvenc", RateControl::Bitrate),
                ("h264_qsv", RateControl::Bitrate),
                ("h264_amf", RateControl::Bitrate),
                ("h264_videotoolbox", RateControl::Bitrate),
                ("h264_mf", RateControl::Bitrate),
                ("libopenh264", RateControl::Bitrate),
            ],
            VideoOutputFormat::WebmVp9 => &[
                ("libvpx-vp9", RateControl::Crf),
                ("vp9_qsv", RateControl::Bitrate),
            ],
        }
    }

    /// CRF の実用レンジはコデックごとに違う。同じ数値でも意味が変わる（`D-19`）。
    fn crf(self, preset: QualityPreset) -> u32 {
        match self {
            VideoOutputFormat::Mp4H264 => match preset {
                QualityPreset::High => 18,
                QualityPreset::Standard => 23,
                QualityPreset::Small => 28,
                QualityPreset::Smallest => 32,
            },
            VideoOutputFormat::WebmVp9 => match preset {
                QualityPreset::High => 28,
                QualityPreset::Standard => 33,
                QualityPreset::Small => 38,
                QualityPreset::Smallest => 43,
            },
        }
    }

    fn crf_max(self) -> u32 {
        match self {
            VideoOutputFormat::Mp4H264 => 51,
            VideoOutputFormat::WebmVp9 => 63,
        }
    }

    /// ビットレート指定経路での係数。VP9 は同じ体感画質をより少ないビットで出せる。
    fn bitrate_scale(self) -> f64 {
        match self {
            VideoOutputFormat::Mp4H264 => 1.0,
            VideoOutputFormat::WebmVp9 => 0.65,
        }
    }

    /// そのままコンテナへ入れられる音声コデック。
    fn audio_passthrough(self) -> &'static [&'static str] {
        match self {
            VideoOutputFormat::Mp4H264 => &["aac", "mp3", "alac"],
            VideoOutputFormat::WebmVp9 => &["opus", "vorbis"],
        }
    }
}

/// 再エンコード時に使う音声エンコーダを選ぶ。
///
/// WebM は AAC を入れられないため Opus を使う。`libopus` が無いビルドでは
/// 内蔵の実験的エンコーダに `-strict -2` を付けて使う。
fn audio_encoder(
    format: VideoOutputFormat,
    encoders: &HashSet<String>,
) -> Result<(&'static str, bool)> {
    match format {
        VideoOutputFormat::Mp4H264 => Ok(("aac", false)),
        VideoOutputFormat::WebmVp9 => {
            if encoders.contains("libopus") {
                Ok(("libopus", false))
            } else if encoders.contains("opus") {
                Ok(("opus", true))
            } else {
                Err(anyhow!("この FFmpeg には Opus エンコーダがありません。"))
            }
        }
    }
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
    /// コンテナに合う音声コデックへ再エンコードする。MP4 は AAC、WebM は Opus。
    Reencode,
    Remove,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoSettings {
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
    /// このファイルの処理にかかった時間。コデックごとの速度差を結果から比べられるようにする。
    elapsed_ms: u64,
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

/// ビルドに含まれている候補を優先順に並べる。
fn candidate_order(
    format: VideoOutputFormat,
    encoders: &HashSet<String>,
) -> Vec<(&'static str, RateControl)> {
    format
        .candidates()
        .iter()
        .filter(|(name, _)| encoders.contains(*name))
        .map(|(name, rate)| (*name, *rate))
        .collect()
}

fn encoder_cache() -> &'static Mutex<HashMap<String, bool>> {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// エンコーダがこの環境で実際に動くか確かめる。
///
/// 「ビルドに含まれている」ことと「動く」ことは別問題。LGPL ビルドには
/// `h264_nvenc` / `h264_qsv` / `h264_amf` がすべて含まれるが、対応する GPU が
/// なければ初期化に失敗する。`-encoders` の一覧だけで選ぶと、多くの環境で
/// 毎回エンコードに失敗することになるため、1 フレームだけ試す。
fn encoder_works(ffmpeg: &Path, encoder: &str) -> bool {
    let key = format!("{}::{}", ffmpeg.display(), encoder);
    if let Some(cached) = encoder_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).copied())
    {
        return cached;
    }

    let works = command(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:s=256x144:d=0.1",
            "-frames:v",
            "1",
            "-c:v",
            encoder,
            "-f",
            "null",
            "-",
        ])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    if let Ok(mut cache) = encoder_cache().lock() {
        cache.insert(key, works);
    }
    works
}

/// 実際に動く候補のうち、最も優先度の高いものを選ぶ。
fn pick_encoder(
    format: VideoOutputFormat,
    tools: &FfmpegTools,
) -> Option<(&'static str, RateControl)> {
    candidate_order(format, &tools.encoders)
        .into_iter()
        .find(|(name, _)| encoder_works(&tools.ffmpeg, name))
}

/// CRF 指定のオプションはエンコーダごとに形が違う。
fn crf_args(encoder: &str, crf: u32) -> Vec<String> {
    match encoder {
        // libvpx は -b:v 0 を併記しないと CRF が効かない。
        // cpu-used の既定値は極端に遅いため、実用的な速度へ寄せる。
        "libvpx-vp9" => vec![
            "-crf".into(),
            crf.to_string(),
            "-b:v".into(),
            "0".into(),
            "-row-mt".into(),
            "1".into(),
            "-deadline".into(),
            "good".into(),
            "-cpu-used".into(),
            "2".into(),
        ],
        _ => vec![
            "-crf".into(),
            crf.to_string(),
            "-preset".into(),
            "medium".into(),
        ],
    }
}

fn command(program: &Path) -> Command {
    // コンソール窓の抑止は Windows だけなので、他では mut が要らない。
    #[cfg_attr(not(windows), allow(unused_mut))]
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

/// ffprobe の `format_name` は多重化された候補を返す。
///
/// MP4 なら `mov,mp4,m4a,3gp,3g2,mj2` のように並ぶため、先頭を採ると .mp4 が
/// `MOV` と表示される。拡張子が候補に含まれていればそれを優先し、含まれない場合
/// だけ先頭へ落とす。
fn container_label(format_name: &str, path: &Path) -> String {
    let names: Vec<&str> = format_name
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    let first = names.first().copied().unwrap_or("unknown");
    if names.len() < 2 {
        return first.to_uppercase();
    }

    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    // m4v / 3gpp のように、拡張子と多重化名が一致しない綴りを寄せる。
    let alias = match ext.as_str() {
        "m4v" | "m4a" => "mp4",
        "3gpp" => "3gp",
        "qt" => "mov",
        other => other,
    };
    names
        .iter()
        .find(|name| **name == alias)
        .copied()
        .unwrap_or(first)
        .to_uppercase()
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

/// PATH に頼れないときの既知の置き場所。
///
/// Finder から起動した .app は launchd の最小 PATH しか継承しないため、Homebrew や
/// /usr/local に入れた ffmpeg が PATH 経由では見つからない。同梱バイナリが無い
/// 環境（開発中の macOS）でも動くように、既知のパスを最後に試す。
fn well_known_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        return Vec::new();
    }
    ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
        .iter()
        .map(|dir| PathBuf::from(dir).join("ffmpeg"))
        .collect()
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
    for candidate in well_known_candidates() {
        attempts.push((candidate, "system"));
    }

    let mut last_error = None;
    for (candidate, source) in attempts {
        // 明示指定と PATH 以外は、存在しないものを試さない。
        if (source == "bundled" || source == "system") && !candidate.exists() {
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

/// 出力形式ごとの利用可否。UI で選べる形式と CRF の有効・無効を決める。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoFormatSupport {
    /// TS 側の VideoOutputFormat と同じ文字列。
    format: &'static str,
    available: bool,
    encoder: Option<String>,
    /// "crf" | "bitrate"
    rate_control: Option<String>,
    crf_max: u32,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoEnvironment {
    available: bool,
    ffmpeg_path: Option<String>,
    ffprobe_path: Option<String>,
    version: Option<String>,
    source: Option<String>,
    formats: Vec<VideoFormatSupport>,
    message: Option<String>,
}

const OUTPUT_FORMATS: &[(VideoOutputFormat, &str)] = &[
    (VideoOutputFormat::Mp4H264, "mp4H264"),
    (VideoOutputFormat::WebmVp9, "webmVp9"),
];

fn format_support(tools: &FfmpegTools) -> Vec<VideoFormatSupport> {
    OUTPUT_FORMATS
        .iter()
        .map(|(format, name)| {
            let picked = pick_encoder(*format, tools);
            let audio = audio_encoder(*format, &tools.encoders);
            let present = candidate_order(*format, &tools.encoders);
            let message = match (&picked, &audio) {
                (None, _) if present.is_empty() => Some(format!(
                    "この FFmpeg には {} 用のエンコーダがありません。",
                    format.video_codec_name().to_uppercase()
                )),
                // 候補はあるが、対応するハードウェアが無いなどで初期化できない。
                (None, _) => Some(format!(
                    "{} 用のエンコーダ（{}）はこの環境では利用できません。",
                    format.video_codec_name().to_uppercase(),
                    present
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(" / ")
                )),
                (_, Err(error)) => Some(format!("{error:#}")),
                _ => None,
            };
            VideoFormatSupport {
                format: name,
                available: picked.is_some() && audio.is_ok(),
                encoder: picked.map(|(encoder, _)| encoder.to_string()),
                rate_control: picked.map(|(_, rate)| rate.as_str().to_string()),
                crf_max: format.crf_max(),
                message,
            }
        })
        .collect()
}

#[tauri::command]
pub(crate) async fn video_environment(ffmpeg_path: Option<String>) -> Result<VideoEnvironment, String> {
    tauri::async_runtime::spawn_blocking(move || match resolve_tools(ffmpeg_path.as_deref()) {
        Ok(tools) => {
            let formats = format_support(&tools);
            VideoEnvironment {
                available: formats.iter().any(|entry| entry.available),
                ffmpeg_path: Some(tools.ffmpeg.to_string_lossy().to_string()),
                ffprobe_path: Some(tools.ffprobe.to_string_lossy().to_string()),
                version: Some(tools.version),
                source: Some(tools.source.to_string()),
                message: if formats.iter().any(|entry| entry.available) {
                    None
                } else {
                    Some("この FFmpeg には利用できる映像エンコーダがありません。".to_string())
                },
                formats,
            }
        }
        Err(error) => VideoEnvironment {
            available: false,
            ffmpeg_path: None,
            ffprobe_path: None,
            version: None,
            source: None,
            formats: Vec::new(),
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

    let container = container_label(
        probe
            .get("format")
            .and_then(|format| format.get("format_name"))
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        path,
    );

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
        format_label: format!("{container} / {video_codec}"),
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
    let bits = f64::from(width)
        * f64::from(height)
        * fps
        * settings.quality_preset.bits_per_pixel()
        * settings.output_format.bitrate_scale();
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
    encoder: &'static str,
    rate: RateControl,
    output: &Path,
) -> Result<EncodePlan> {
    let format = settings.output_format;

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
            let crf = settings
                .crf_override
                .map(|value| value.min(format.crf_max()))
                .unwrap_or_else(|| format.crf(settings.quality_preset));
            args.extend(crf_args(encoder, crf));
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
            .map(|codec| format.audio_passthrough().contains(&codec))
            .unwrap_or(false);
        let (audio_codec, needs_strict) = audio_encoder(format, &tools.encoders)?;
        match settings.audio_mode {
            AudioMode::Copy if passthrough => {
                args.push("-c:a".into());
                args.push("copy".into());
            }
            AudioMode::Copy | AudioMode::Reencode => {
                if matches!(settings.audio_mode, AudioMode::Copy) {
                    warnings.push(format!(
                        "{} は {} へそのまま入れられないため {} へ変換しました",
                        entry.audio_codec.as_deref().unwrap_or("この音声"),
                        format.extension().to_uppercase(),
                        audio_codec
                    ));
                }
                args.push("-c:a".into());
                args.push(audio_codec.into());
                args.push("-b:a".into());
                args.push(format!("{}k", settings.audio_bitrate_kbps));
                if needs_strict {
                    args.push("-strict".into());
                    args.push("-2".into());
                }
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

    // faststart は mp4 muxer 固有のオプション。webm へ渡すとエラーになる。
    if matches!(format, VideoOutputFormat::Mp4H264) {
        args.push("-movflags".into());
        args.push("+faststart".into());
    }
    args.push("-f".into());
    args.push(format.muxer().into());
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
        && entry.video_codec == settings.output_format.video_codec_name()
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
    // エンコーダの確認はプロセス起動を伴うので、バッチの先頭で 1 回だけ行う。
    let format = request.settings.output_format;
    let (encoder, rate) = pick_encoder(format, &tools).ok_or_else(|| {
        anyhow!(
            "{} 用のエンコーダがこの環境では利用できません。",
            format.video_codec_name().to_uppercase()
        )
    })?;
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

        let started = Instant::now();
        let mut result = catch_task_panic("video processing", || {
            process_one(
                &window,
                &control,
                &tools,
                entry,
                &request.settings,
                encoder,
                rate,
                &output_root,
                index,
                total,
            )
        })
        .unwrap_or_else(|error| failure_item(entry, format!("{error:#}")));
        // 成功・失敗・中断のどれでも同じ計り方になるよう、ここで入れる。
        result.elapsed_ms = started.elapsed().as_millis() as u64;

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
        elapsed_ms: 0,
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
    encoder: &'static str,
    rate: RateControl,
    output_root: &Path,
    index: usize,
    total: usize,
) -> Result<VideoResultItem> {
    let output_path = build_output_path_from(
        &entry.relative_path,
        output_root,
        settings.output_format.extension(),
        settings.overwrite,
    )?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory: {}", parent.display()))?;
    }

    // 出力は一時ファイルへ書き、成功時にリネームする。
    // 途中のファイルを完成品として残さないため（`D-20`）。
    let temp_path =
        output_path.with_extension(format!("{}.part", settings.output_format.extension()));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }

    let plan = build_encode_plan(entry, settings, tools, encoder, rate, &temp_path)?;
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
            elapsed_ms: 0,
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
        // Windows の rename は宛先があると失敗するため、上書き時は先に消す。
        if output_path.exists() {
            fs::remove_file(&output_path).with_context(|| {
                format!("failed to replace existing output: {}", output_path.display())
            })?;
        }
        if let Err(error) = fs::rename(&temp_path, &output_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(anyhow!(error)
                .context(format!("failed to finalize output: {}", output_path.display())));
        }
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
            format!("{} / {}", settings.output_format.extension(), plan.encoder)
        }),
        original_size: entry.file_size,
        optimized_size: Some(final_size),
        saved_size: Some(saved),
        saved_percent,
        width,
        height,
        duration_sec: entry.duration_sec,
        elapsed_ms: 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(width: u32, height: u32, fps: f64) -> VideoInputEntry {
        VideoInputEntry {
            id: "id".into(),
            source_path: "input.mp4".into(),
            root_path: ".".into(),
            relative_path: "input.mp4".into(),
            file_name: "input.mp4".into(),
            format_label: "MP4 / h264".into(),
            video_codec: "h264".into(),
            audio_codec: Some("aac".into()),
            file_size: 1_000_000,
            width: Some(width),
            height: Some(height),
            duration_sec: Some(60.0),
            fps: Some(fps),
            variable_frame_rate: false,
            bit_rate: Some(4_000_000),
            rotation: None,
            has_audio: true,
            audio_track_count: 1,
            subtitle_track_count: 0,
            hdr: false,
            warnings: Vec::new(),
        }
    }

    /// .mp4 の format_name は mov から始まる。拡張子が候補にあればそちらを採る。
    #[test]
    fn container_label_prefers_the_extension() {
        let mp4 = Path::new("/tmp/C0370.MP4");
        assert_eq!(container_label("mov,mp4,m4a,3gp,3g2,mj2", mp4), "MP4");

        let mov = Path::new("/tmp/clip.mov");
        assert_eq!(container_label("mov,mp4,m4a,3gp,3g2,mj2", mov), "MOV");

        // 候補に無い拡張子は先頭へ落とす。
        let unknown = Path::new("/tmp/clip.bin");
        assert_eq!(container_label("mov,mp4,m4a,3gp,3g2,mj2", unknown), "MOV");

        // m4v は mp4 として多重化される。
        let m4v = Path::new("/tmp/clip.m4v");
        assert_eq!(container_label("mov,mp4,m4a,3gp,3g2,mj2", m4v), "MP4");

        // 候補が 1 つだけなら拡張子は見ない。
        let webm = Path::new("/tmp/clip.webm");
        assert_eq!(container_label("matroska,webm", webm), "WEBM");
        assert_eq!(container_label("avi", Path::new("/tmp/clip.avi")), "AVI");
    }

    fn settings() -> VideoSettings {
        VideoSettings {
            output_format: VideoOutputFormat::Mp4H264,
            output_mode: OutputMode::Custom,
            custom_output_dir: Some("out".into()),
            overwrite: false,
            resize: ResizeSettings {
                mode: ResizeMode::None,
                value: None,
                unit: ResizeUnit::Px,
            },
            quality_preset: QualityPreset::Standard,
            crf_override: None,
            fps_limit: None,
            audio_mode: AudioMode::Copy,
            audio_bitrate_kbps: 128,
            metadata_mode: MetadataMode::Strip,
            timestamps: TimestampSettings {
                preserve_creation_time: false,
                preserve_last_write_time: false,
            },
            ffmpeg_path: None,
        }
    }

    /// リサイズなしでも偶数へ丸める式を出す。奇数寸法の入力で yuv420p が失敗するため。
    #[test]
    fn scale_filter_forces_even_dimensions_without_resize() {
        let filter = scale_filter(&settings().resize);
        assert!(filter.contains("trunc(iw/2)*2"), "{filter}");
        assert!(filter.contains("trunc(ih/2)*2"), "{filter}");
    }

    /// px 指定は min() で抑えるため、拡大にはならない。
    #[test]
    fn scale_filter_never_upscales() {
        let mut resize = settings().resize;
        resize.mode = ResizeMode::Width;
        resize.value = Some(640);
        assert_eq!(
            scale_filter(&resize),
            "scale=w='trunc(min(iw,640)/2)*2':h=-2"
        );
    }

    /// 長辺指定は縦横のどちらが長いかを ffmpeg 側で判定させる。
    /// 回転メタデータつきの動画では、Rust 側の寸法と表示寸法が入れ替わるため。
    #[test]
    fn scale_filter_long_edge_branches_on_orientation() {
        let mut resize = settings().resize;
        resize.mode = ResizeMode::LongEdge;
        resize.value = Some(1080);
        let filter = scale_filter(&resize);
        assert!(filter.contains("if(gte(iw,ih)"), "{filter}");
        assert!(filter.contains("min(iw,1080)"), "{filter}");
        assert!(filter.contains("min(ih,1080)"), "{filter}");
    }

    /// % 指定は基準に関係なく一様縮小。100 を超える値は受け付けない。
    #[test]
    fn scale_filter_percent_is_capped_at_100() {
        let mut resize = settings().resize;
        resize.mode = ResizeMode::LongEdge;
        resize.unit = ResizeUnit::Percent;
        resize.value = Some(150);
        assert_eq!(scale_filter(&resize), "scale=w='trunc(iw*100/200)*2':h=-2");
    }

    /// 入力より高い fps は指定しない。フレーム複製で太るだけになる。
    #[test]
    fn video_filter_skips_fps_when_source_is_slower() {
        let mut config = settings();
        config.fps_limit = Some(60);
        let filter = video_filter(&entry(1920, 1080, 30.0), &config);
        assert!(!filter.contains("fps="), "{filter}");

        config.fps_limit = Some(24);
        let filter = video_filter(&entry(1920, 1080, 30.0), &config);
        assert!(filter.starts_with("fps=24,"), "{filter}");
    }

    /// 可変フレームレートは、上限指定がなくても平均レートで固定化する。
    #[test]
    fn video_filter_pins_variable_frame_rate() {
        let mut source = entry(1280, 720, 29.97);
        source.variable_frame_rate = true;
        let filter = video_filter(&source, &settings());
        assert!(filter.starts_with("fps=29.970,"), "{filter}");
    }

    /// 回転つきの動画では、ビットレート見積りに表示寸法を使う。
    #[test]
    fn bitrate_uses_display_dimensions_for_rotated_input() {
        let mut source = entry(1920, 1080, 30.0);
        let upright = target_bitrate_kbps(&source, &settings());
        source.rotation = Some(90);
        let rotated = target_bitrate_kbps(&source, &settings());
        // 縦横が入れ替わってもピクセル数は同じなので、見積りは変わらない。
        assert_eq!(upright, rotated);
    }

    /// 品質プリセットが下がるほどビットレートも下がる。
    #[test]
    fn bitrate_follows_quality_preset() {
        let source = entry(1920, 1080, 30.0);
        let mut config = settings();
        config.quality_preset = QualityPreset::High;
        let high = target_bitrate_kbps(&source, &config);
        config.quality_preset = QualityPreset::Smallest;
        let smallest = target_bitrate_kbps(&source, &config);
        assert!(high > smallest, "high={high} smallest={smallest}");
    }

    /// リサイズすると見積りも下がる。
    #[test]
    fn bitrate_drops_when_resized() {
        let source = entry(1920, 1080, 30.0);
        let mut config = settings();
        let full = target_bitrate_kbps(&source, &config);
        config.resize.mode = ResizeMode::LongEdge;
        config.resize.value = Some(960);
        let half = target_bitrate_kbps(&source, &config);
        assert!(half < full, "full={full} half={half}");
    }

    /// libx264 があれば CRF、なければビットレート指定になる。
    #[test]
    fn encoder_choice_prefers_crf_capable_encoder() {
        let mut encoders = HashSet::new();
        encoders.insert("h264_mf".to_string());
        assert_eq!(
            candidate_order(VideoOutputFormat::Mp4H264, &encoders).first().copied(),
            Some(("h264_mf", RateControl::Bitrate))
        );

        encoders.insert("libx264".to_string());
        assert_eq!(
            candidate_order(VideoOutputFormat::Mp4H264, &encoders).first().copied(),
            Some(("libx264", RateControl::Crf))
        );

        assert!(candidate_order(VideoOutputFormat::Mp4H264, &HashSet::new()).is_empty());
    }

    /// VP9 と H.264 でエンコーダ候補が混ざらないこと。
    #[test]
    fn encoder_choice_is_per_output_format() {
        let mut encoders = HashSet::new();
        encoders.insert("libx264".to_string());
        assert!(candidate_order(VideoOutputFormat::WebmVp9, &encoders).is_empty());

        encoders.insert("libvpx-vp9".to_string());
        assert_eq!(
            candidate_order(VideoOutputFormat::WebmVp9, &encoders).first().copied(),
            Some(("libvpx-vp9", RateControl::Crf))
        );
    }

    /// ハードウェアが無い環境向けに、候補は複数残しておく。
    /// 1 つしか候補がないと、その環境では H.264 が一切出せなくなる。
    #[test]
    fn hardware_candidates_have_software_fallbacks() {
        let names: Vec<&str> = VideoOutputFormat::Mp4H264
            .candidates()
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert!(names.contains(&"h264_mf"), "{names:?}");
        assert!(names.contains(&"libopenh264"), "{names:?}");
        // ハードウェア依存のものより後ろにあること。
        let mf = names.iter().position(|name| *name == "h264_mf").unwrap();
        let nvenc = names.iter().position(|name| *name == "h264_nvenc").unwrap();
        assert!(nvenc < mf, "{names:?}");
    }

    /// libvpx は -b:v 0 を併記しないと CRF が効かない。
    #[test]
    fn vp9_crf_args_pin_bitrate_to_zero() {
        let args = crf_args("libvpx-vp9", 33);
        assert!(args.windows(2).any(|pair| pair == ["-b:v", "0"]), "{args:?}");
        assert!(args.windows(2).any(|pair| pair == ["-crf", "33"]), "{args:?}");
        // preset は libvpx にないオプション。
        assert!(!args.iter().any(|arg| arg == "-preset"), "{args:?}");
    }

    /// 同じプリセットでも、コデックごとに CRF の値が変わる。
    #[test]
    fn crf_differs_between_codecs() {
        assert_eq!(VideoOutputFormat::Mp4H264.crf(QualityPreset::Standard), 23);
        assert_eq!(VideoOutputFormat::WebmVp9.crf(QualityPreset::Standard), 33);
        assert_eq!(VideoOutputFormat::Mp4H264.crf_max(), 51);
        assert_eq!(VideoOutputFormat::WebmVp9.crf_max(), 63);
    }

    /// WebM は AAC を入れられないので Opus を選ぶ。
    #[test]
    fn audio_encoder_follows_container() {
        let mut encoders = HashSet::new();
        encoders.insert("aac".to_string());
        assert_eq!(
            audio_encoder(VideoOutputFormat::Mp4H264, &encoders).unwrap(),
            ("aac", false)
        );
        assert!(audio_encoder(VideoOutputFormat::WebmVp9, &encoders).is_err());

        encoders.insert("opus".to_string());
        assert_eq!(
            audio_encoder(VideoOutputFormat::WebmVp9, &encoders).unwrap(),
            ("opus", true)
        );

        encoders.insert("libopus".to_string());
        assert_eq!(
            audio_encoder(VideoOutputFormat::WebmVp9, &encoders).unwrap(),
            ("libopus", false)
        );
    }

    /// AAC 音声を WebM へは通せない。逆も同じ。
    #[test]
    fn audio_passthrough_is_per_container() {
        assert!(VideoOutputFormat::Mp4H264.audio_passthrough().contains(&"aac"));
        assert!(!VideoOutputFormat::Mp4H264.audio_passthrough().contains(&"opus"));
        assert!(VideoOutputFormat::WebmVp9.audio_passthrough().contains(&"opus"));
        assert!(!VideoOutputFormat::WebmVp9.audio_passthrough().contains(&"aac"));
    }

    /// VP9 は同じ体感画質をより少ないビットで出せる前提の係数。
    #[test]
    fn vp9_bitrate_estimate_is_lower_than_h264() {
        let source = entry(1920, 1080, 30.0);
        let mut config = settings();
        let h264 = target_bitrate_kbps(&source, &config);
        config.output_format = VideoOutputFormat::WebmVp9;
        let vp9 = target_bitrate_kbps(&source, &config);
        assert!(vp9 < h264, "h264={h264} vp9={vp9}");
    }

    /// 分数表記の fps を読めること。0 除算で落ちないこと。
    #[test]
    fn parses_frame_rate_fractions() {
        assert_eq!(parse_rational(Some("30/1")), Some(30.0));
        assert!((parse_rational(Some("30000/1001")).unwrap() - 29.97).abs() < 0.01);
        assert_eq!(parse_rational(Some("0/0")), None);
        assert_eq!(parse_rational(None), None);
    }

    /// 元コピーへのフォールバックは、何も変えない指定のときだけ。
    #[test]
    fn source_copy_only_when_nothing_changes() {
        let source = entry(1920, 1080, 30.0);
        let mut config = settings();
        assert!(is_passthrough_request(&source, &config));

        config.resize.mode = ResizeMode::Width;
        config.resize.value = Some(1280);
        assert!(!is_passthrough_request(&source, &config));

        config = settings();
        config.fps_limit = Some(24);
        assert!(!is_passthrough_request(&source, &config));

        config = settings();
        config.audio_mode = AudioMode::Reencode;
        assert!(!is_passthrough_request(&source, &config));

        // 出力形式が変われば、H.264 入力でもコピーにはならない。
        config = settings();
        config.output_format = VideoOutputFormat::WebmVp9;
        assert!(!is_passthrough_request(&source, &config));
    }
}
