use super::*;
use std::collections::HashMap;

fn repo_sample_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("samples")
        .join("input")
}

/// 既定の入力フォルダ。実機のサンプルを見る確認用なので、ホームから組み立てる。
fn desktop_sample_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(
        PathBuf::from(home)
            .join("Desktop")
            .join("@StorageSlim")
            .join("input"),
    )
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("storageslim-{name}-{}", std::process::id()));
    if path.exists() {
        fs::remove_dir_all(&path).unwrap();
    }
    fs::create_dir_all(&path).unwrap();
    path
}

fn default_test_settings(output_root: &Path) -> BatchSettings {
    BatchSettings {
        output_format: OutputFormat::Original,
        output_mode: OutputMode::Custom,
        custom_output_dir: Some(output_root.to_string_lossy().to_string()),
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
    }
}

#[test]
fn inspect_repo_samples_reports_expected_flags() {
    let sample_dir = repo_sample_dir();
    assert!(sample_dir.exists(), "sample directory is missing: {}", sample_dir.display());

    let response = inspect_inputs_impl(vec![sample_dir.to_string_lossy().to_string()]).unwrap();
    assert!(response.skipped.is_empty());
    assert_eq!(response.entries.len(), 12);

    let by_name: HashMap<_, _> = response
        .entries
        .iter()
        .map(|entry| (entry.file_name.as_str(), entry))
        .collect();

    assert!(by_name["sample-animated.gif"].animated);
    assert!(!by_name["sample-static.gif"].animated);
    assert!(!by_name["sample-avif.avif"].runtime_supported);
    assert!(by_name["sample-heic.heic"].runtime_supported);
    assert!(by_name["sample-heif.heif"].runtime_supported);
    assert!(by_name["sample-photo.jpg"].width.is_some());
    assert!(by_name["sample-graphic.png"].height.is_some());
    assert!(by_name["sample-webp.webp"].runtime_supported);

    // PSD は統合画像を持つ RGB 8bit だけを対象にする。範囲外のものは寸法を
    // 読んだうえで実行対象から外し、理由を警告として見せる。
    assert!(by_name["sample-psd.psd"].runtime_supported);
    assert!(by_name["sample-psd-rle.psd"].runtime_supported);
    assert_eq!(by_name["sample-psd.psd"].width, Some(160));
    assert!(!by_name["sample-psd-cmyk.psd"].runtime_supported);
    assert!(!by_name["sample-psd-16bit.psd"].runtime_supported);
    assert!(by_name["sample-psd-cmyk.psd"].warnings[0].contains("CMYK"));
    assert!(by_name["sample-psd-16bit.psd"].warnings[0].contains("16bit"));
}

#[test]
fn inspect_desktop_samples_reports_expected_flags() {
    // 実機のデスクトップに一式を置いた環境だけで意味がある確認。
    // 置いていない環境（CI や別の OS）では落とさず飛ばす。
    let Some(sample_dir) = desktop_sample_dir().filter(|dir| dir.exists()) else {
        eprintln!("skipped: desktop sample directory is not present");
        return;
    };

    let response = inspect_inputs_impl(vec![sample_dir.to_string_lossy().to_string()]).unwrap();
    assert!(response.skipped.is_empty(), "skipped: {:?}", response.skipped);
    assert_eq!(response.entries.len(), 8);

    let by_name: HashMap<_, _> = response
        .entries
        .iter()
        .map(|entry| (entry.file_name.as_str(), entry))
        .collect();

    assert!(by_name["sample-animated.gif"].animated);
    assert!(!by_name["sample-static.gif"].animated);
    assert!(!by_name["sample-avif.avif"].runtime_supported);
    assert!(by_name["sample-heic.heic"].runtime_supported);
    assert!(by_name["sample-heif.heif"].runtime_supported);
    assert!(by_name["sample-photo.jpg"].width.is_some());
    assert!(by_name["sample-graphic.png"].height.is_some());
    assert!(by_name["sample-webp.webp"].runtime_supported);
}

#[test]
fn repo_samples_process_or_fail_as_expected() {
    let sample_dir = repo_sample_dir();
    let response = inspect_inputs_impl(vec![sample_dir.to_string_lossy().to_string()]).unwrap();
    assert!(response.skipped.is_empty());
    let output_root = temp_dir("repo-samples-output");
    let settings = default_test_settings(&output_root);
    fs::create_dir_all(&output_root).unwrap();

    let mut outcomes = HashMap::new();
    for entry in &response.entries {
        let result = process_one(entry, &settings, &output_root);
        outcomes.insert(entry.file_name.clone(), result);
    }

    assert!(outcomes["sample-photo.jpg"].as_ref().is_ok());
    assert!(outcomes["sample-graphic.png"].as_ref().is_ok());
    assert!(outcomes["sample-webp.webp"].as_ref().is_ok());
    assert!(outcomes["sample-avif.avif"].as_ref().is_err());
    assert!(outcomes["sample-static.gif"].as_ref().is_ok());
    assert!(outcomes["sample-animated.gif"].as_ref().is_ok());
    assert!(outcomes["sample-heic.heic"].as_ref().is_ok());
    assert!(outcomes["sample-heif.heif"].as_ref().is_ok());
    // PSD へは書き戻せないため、既定のオリジナル維持では必ず失敗する。
    assert!(outcomes["sample-psd.psd"].as_ref().is_err());
    assert!(outcomes["sample-psd-cmyk.psd"].as_ref().is_err());
    assert!(outcomes["sample-psd-16bit.psd"].as_ref().is_err());
}

#[test]
fn psd_samples_convert_to_webp() {
    let sample_dir = repo_sample_dir();
    let response = inspect_inputs_impl(vec![sample_dir.to_string_lossy().to_string()]).unwrap();
    let output_root = temp_dir("psd-samples-output");
    let mut settings = default_test_settings(&output_root);
    settings.output_format = OutputFormat::Webp;

    for name in ["sample-psd.psd", "sample-psd-rle.psd"] {
        let entry = response
            .entries
            .iter()
            .find(|entry| entry.file_name == name)
            .unwrap_or_else(|| panic!("sample is missing: {name}"));
        let result = process_one(entry, &settings, &output_root).unwrap();
        assert_eq!((result.width, result.height), (Some(160), Some(90)));
        assert!(result.optimized_size.unwrap() < entry.file_size);
    }
}

#[test]
fn psd_rle_and_raw_decode_to_the_same_pixels() {
    // Photoshop は RLE で書き出す。ImageMagick が作るサンプルは無圧縮なので、
    // 同じ絵の両方を突き合わせて展開結果が一致することを見る。
    let sample_dir = repo_sample_dir();
    let limit = decode_limit_bytes(DECODE_LIMIT_DEFAULT_MB);
    let raw = psd::decode_composite(&sample_dir.join("sample-psd.psd"), limit).unwrap();
    let rle = psd::decode_composite(&sample_dir.join("sample-psd-rle.psd"), limit).unwrap();

    assert_eq!(raw.dimensions(), rle.dimensions());
    assert_eq!(raw.to_rgba8().into_raw(), rle.to_rgba8().into_raw());
}
