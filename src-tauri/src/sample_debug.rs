use super::*;
use std::collections::HashMap;

fn repo_sample_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("samples")
        .join("input")
}

fn desktop_sample_dir() -> PathBuf {
    PathBuf::from(r"C:\Users\Owner\Desktop\@StorageSlim\input")
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
fn inspect_desktop_samples_reports_expected_flags() {
    let sample_dir = desktop_sample_dir();
    assert!(sample_dir.exists(), "desktop sample directory is missing: {}", sample_dir.display());

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
}
