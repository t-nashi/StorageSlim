//! 表示方向の回帰検証。実写真はリポジトリへ含めず、任意の環境変数で検証する。

use super::*;

fn settings() -> BatchSettings {
    BatchSettings {
        output_format: OutputFormat::Png,
        output_mode: OutputMode::Custom,
        custom_output_dir: None,
        overwrite: true,
        resize: ResizeSettings {
            mode: ResizeMode::None,
            value: None,
            unit: ResizeUnit::Px,
        },
        quality: QualitySettings {
            jpeg_quality: 95,
            webp_quality: 95.0,
            avif_quality: 90,
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

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "storageslim-orientation-{name}-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn tiff(value: u8, big_endian: bool) -> Vec<u8> {
    // Orientation のみを持つ TIFF。両方のバイトオーダーを検証する。
    if big_endian {
        vec![
            77, 77, 0, 42, 0, 0, 0, 8, 0, 1, 1, 18, 0, 3, 0, 0, 0, 1, 0, value, 0, 0, 0, 0, 0, 0,
        ]
    } else {
        vec![
            73, 73, 42, 0, 8, 0, 0, 0, 1, 0, 18, 1, 3, 0, 1, 0, 0, 0, value, 0, 0, 0, 0, 0, 0, 0,
        ]
    }
}

fn write_source(path: &Path, image: &DynamicImage, format: InputFormat, exif: &[u8]) {
    if matches!(format, InputFormat::Psd) {
        // 統合 RGB 8bit / 無圧縮 PSD。EXIF は画像リソース 1058 に置く。
        let mut resources = b"8BIM".to_vec();
        resources.extend_from_slice(&1058u16.to_be_bytes());
        resources.extend_from_slice(&[0, 0]);
        resources.extend_from_slice(&(exif.len() as u32).to_be_bytes());
        resources.extend_from_slice(exif);
        if exif.len() % 2 != 0 {
            resources.push(0);
        }
        let mut bytes = b"8BPS".to_vec();
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&[0; 6]);
        bytes.extend_from_slice(&3u16.to_be_bytes());
        bytes.extend_from_slice(&image.height().to_be_bytes());
        bytes.extend_from_slice(&image.width().to_be_bytes());
        bytes.extend_from_slice(&8u16.to_be_bytes());
        bytes.extend_from_slice(&3u16.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&(resources.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&resources);
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        let rgb = image.to_rgb8();
        for channel in 0..3 {
            bytes.extend(rgb.pixels().map(|p| p[channel]));
        }
        fs::write(path, bytes).unwrap();
    } else {
        let output = match format {
            InputFormat::Jpeg => OutputFormat::Jpeg,
            InputFormat::Png => OutputFormat::Png,
            InputFormat::Webp => OutputFormat::Webp,
            _ => unreachable!(),
        };
        let encoded = encode_static_image(image, &settings(), output).unwrap();
        let bytes = exif::embed(&encoded, output, exif, image.width(), image.height()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

#[test]
fn all_eight_orientations_preserve_pixels_and_metadata_policy() {
    let dir = test_dir("eight");
    let source = DynamicImage::ImageRgba8(RgbaImage::from_fn(3, 2, |x, y| {
        let v = ((y * 3 + x) * 35 + 10) as u8;
        Rgba([v, v, v, 255])
    }));
    // 入力の画素番号 abc/def の正しい表示順。実装の回転 API に依存しない期待値。
    let orders = [
        [0, 1, 2, 3, 4, 5],
        [2, 1, 0, 5, 4, 3],
        [5, 4, 3, 2, 1, 0],
        [3, 4, 5, 0, 1, 2],
        [0, 3, 1, 4, 2, 5],
        [3, 0, 4, 1, 5, 2],
        [5, 2, 4, 1, 3, 0],
        [2, 5, 1, 4, 0, 3],
    ];
    for (format, extension) in [
        (InputFormat::Jpeg, "jpg"),
        (InputFormat::Png, "png"),
        (InputFormat::Webp, "webp"),
        (InputFormat::Psd, "psd"),
    ] {
        for value in 1..=8u8 {
            for big in [false, true] {
                let path = dir.join(format!("input-{extension}-{value}-{big}.{extension}"));
                write_source(&path, &source, format.clone(), &tiff(value, big));
                let entry =
                    inspect_single(&path, &dir, path.file_name().unwrap().to_str().unwrap())
                        .unwrap();
                let dims = if value >= 5 { (2, 3) } else { (3, 2) };
                assert_eq!((entry.width, entry.height), (Some(dims.0), Some(dims.1)));
                let raw = decode_input_image(&entry, DECODE_LIMIT_DEFAULT_MB)
                    .unwrap()
                    .to_rgba8();
                let expected: Vec<_> = orders[usize::from(value - 1)]
                    .iter()
                    .map(|index| *raw.get_pixel(index % 3, index / 3))
                    .collect();
                for mode in [
                    MetadataMode::Strip,
                    MetadataMode::DateOnly,
                    MetadataMode::Keep,
                ] {
                    let mut config = settings();
                    config.metadata_mode = mode;
                    let result = process_one(&entry, &config, &dir.join("output")).unwrap();
                    let bytes = fs::read(result.output_path.unwrap()).unwrap();
                    let output = image::load_from_memory(&bytes).unwrap().to_rgba8();
                    assert_eq!(output.dimensions(), dims, "{path:?}");
                    assert_eq!(
                        output.pixels().copied().collect::<Vec<_>>(),
                        expected,
                        "{path:?}"
                    );
                    assert_eq!((result.width, result.height), (Some(dims.0), Some(dims.1)));
                    let metadata = exif::extract(&bytes, &InputFormat::Png);
                    if matches!(config.metadata_mode, MetadataMode::Strip) {
                        assert!(metadata.is_none());
                    } else {
                        assert_eq!(
                            Orientation::from_exif_chunk(&metadata.unwrap()),
                            Some(Orientation::NoTransforms)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn rotated_input_resizes_in_display_coordinates_for_every_output_format() {
    let dir = test_dir("outputs");
    let source = DynamicImage::ImageRgba8(RgbaImage::from_fn(60, 40, |x, y| {
        let v = if x < 30 {
            if y < 20 {
                20
            } else {
                80
            }
        } else if y < 20 {
            150
        } else {
            230
        };
        Rgba([v, v, v, 255])
    }));
    let path = dir.join("source.png");
    write_source(&path, &source, InputFormat::Png, &tiff(8, false));
    let entry = inspect_single(&path, &dir, "source.png").unwrap();
    for format in [
        OutputFormat::Jpeg,
        OutputFormat::Png,
        OutputFormat::Webp,
        OutputFormat::Gif,
        OutputFormat::Avif,
    ] {
        for mode in [
            MetadataMode::Strip,
            MetadataMode::DateOnly,
            MetadataMode::Keep,
        ] {
            for (resize_mode, unit, value, dims) in [
                (ResizeMode::Width, ResizeUnit::Px, 20, (20, 30)),
                (ResizeMode::Height, ResizeUnit::Px, 30, (20, 30)),
                (ResizeMode::LongEdge, ResizeUnit::Px, 30, (20, 30)),
                (ResizeMode::Width, ResizeUnit::Percent, 50, (20, 30)),
                (ResizeMode::None, ResizeUnit::Px, 0, (40, 60)),
            ] {
                let mut config = settings();
                config.output_format = format;
                config.metadata_mode = mode.clone();
                let case_dir = dir.join(format!(
                    "{format:?}-{mode:?}-{value}-{unit:?}-{resize_mode:?}"
                ));
                config.resize = ResizeSettings {
                    mode: resize_mode,
                    value: Some(value),
                    unit,
                };
                let result = process_one(&entry, &config, &case_dir).unwrap();
                let output_path = PathBuf::from(result.output_path.unwrap());
                assert_eq!((result.width, result.height), (Some(dims.0), Some(dims.1)));
                if matches!(format, OutputFormat::Gif | OutputFormat::Avif)
                    && !matches!(mode, MetadataMode::Strip)
                {
                    assert!(result
                        .warnings
                        .iter()
                        .any(|w| w.contains("EXIF を埋め込めない")));
                }
                // このビルドのデコーダは AVIF 非対応。生成した AVIF の画素・寸法は
                // 別デコーダによる検証で確認する。
                if format == OutputFormat::Avif {
                    continue;
                }
                let output = image::open(&output_path).unwrap();
                assert_eq!(output.dimensions(), dims, "{format:?}");
                // 90度反時計回り: 左上=元の右上、右下=元の左下。
                for (x, y, expected) in [
                    (dims.0 / 4, dims.1 / 4, 150i16),
                    (dims.0 * 3 / 4, dims.1 * 3 / 4, 80),
                ] {
                    assert!(
                        (i16::from(output.get_pixel(x, y)[0]) - expected).abs() < 25,
                        "{format:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn invalid_or_missing_orientation_keeps_the_original_pixels() {
    let dir = test_dir("invalid");
    let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(3, 2, Rgba([15, 40, 80, 255])));
    for (index, metadata) in [
        tiff(0, false),
        tiff(9, true),
        b"II\x2a\x00".to_vec(),
        Vec::new(),
    ]
    .iter()
    .enumerate()
    {
        let path = dir.join(format!("source-{index}.png"));
        write_source(&path, &source, InputFormat::Png, metadata);
        let entry =
            inspect_single(&path, &dir, path.file_name().unwrap().to_str().unwrap()).unwrap();
        let result = process_one(&entry, &settings(), &dir.join("output")).unwrap();
        let output = image::open(result.output_path.unwrap()).unwrap();
        assert_eq!(output.to_rgba8(), source.to_rgba8());
    }
}

#[test]
#[ignore = "実写真は STORAGESLIM_ORIENTATION_SAMPLE で指定する"]
fn real_photo_preserves_portrait_orientation() {
    let path = PathBuf::from(
        std::env::var_os("STORAGESLIM_ORIENTATION_SAMPLE").expect("入力写真のパスが必要"),
    );
    let output_root = PathBuf::from(
        std::env::var_os("STORAGESLIM_ORIENTATION_OUTPUT").expect("検証結果の保存先が必要"),
    );
    let entry = inspect_single(
        &path,
        path.parent().unwrap(),
        path.file_name().unwrap().to_str().unwrap(),
    )
    .unwrap();
    assert_eq!((entry.width, entry.height), (Some(4128), Some(6192)));
    for (index, mode) in [
        MetadataMode::Strip,
        MetadataMode::DateOnly,
        MetadataMode::Keep,
    ]
    .into_iter()
    .enumerate()
    {
        let mut config = settings();
        config.output_format = OutputFormat::Jpeg;
        config.quality.jpeg_quality = 82;
        config.metadata_mode = mode;
        config.resize = ResizeSettings {
            mode: ResizeMode::LongEdge,
            value: Some(1500),
            unit: ResizeUnit::Px,
        };
        let result =
            process_one(&entry, &config, &output_root.join(format!("mode-{index}"))).unwrap();
        let bytes = fs::read(result.output_path.as_ref().unwrap()).unwrap();
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().dimensions(),
            (1000, 1500)
        );
        if let Some(metadata) = exif::extract(&bytes, &InputFormat::Jpeg) {
            assert_eq!(
                Orientation::from_exif_chunk(&metadata),
                Some(Orientation::NoTransforms)
            );
        } else {
            assert_eq!(index, 0);
        }
        eprintln!("検証出力: {}", result.output_path.unwrap());
    }
}

/// 同梱 HEIF サンプルへ正式な irot / imir プロパティと関連付けを追加する。
/// 元の圧縮データは変えず、meta の増加分だけ iloc の絶対位置を補正する。
fn transformed_heif(rotation: Option<u8>, mirror: Option<u8>) -> Vec<u8> {
    let mut bytes = include_bytes!("../../samples/input/sample-heic.heic").to_vec();
    fn locate(bytes: &[u8], tag: &[u8]) -> usize {
        bytes.windows(4).position(|w| w == tag).unwrap() - 4
    }
    fn read_u32(bytes: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
    }
    fn add_size(bytes: &mut [u8], at: usize, extra: u32) {
        let size = read_u32(bytes, at) + extra;
        bytes[at..at + 4].copy_from_slice(&size.to_be_bytes());
    }
    let meta = locate(&bytes, b"meta");
    let iloc = locate(&bytes, b"iloc");
    let iprp = locate(&bytes, b"iprp");
    let ipco = locate(&bytes, b"ipco");
    let ipma = locate(&bytes, b"ipma");
    let mut properties = Vec::new();
    let mut associations = Vec::new();
    for (tag, value) in [(b"irot", rotation), (b"imir", mirror)] {
        if let Some(value) = value {
            properties.extend_from_slice(&9u32.to_be_bytes());
            properties.extend_from_slice(tag);
            properties.push(value);
            // サンプルは7プロパティ。追加プロパティは必須として関連付ける。
            associations.push(0x80 | (8 + associations.len() as u8));
        }
    }
    let extra = (properties.len() + associations.len()) as u32;
    // version 0、offset / length / base_offset = 4 byte の固定サンプル構造。
    assert_eq!(&bytes[iloc + 8..iloc + 16], &[0, 0, 0, 0, 0x44, 0x40, 0, 2]);
    for at in [iloc + 20, iloc + 38] {
        let offset = read_u32(&bytes, at) + extra;
        bytes[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    }
    assert_eq!(
        &bytes[ipma + 8..ipma + 19],
        &[0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 4]
    );
    bytes[ipma + 18] += associations.len() as u8;
    add_size(&mut bytes, meta, extra);
    add_size(&mut bytes, iprp, extra);
    add_size(&mut bytes, ipco, properties.len() as u32);
    add_size(&mut bytes, ipma, associations.len() as u32);
    bytes.splice(ipma + 23..ipma + 23, associations);
    bytes.splice(ipma..ipma, properties);
    bytes
}

#[test]
fn heic_and_heif_container_transforms_are_applied_once() {
    let dir = test_dir("heif");
    let source_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../samples/input/sample-heic.heic");
    let baseline = decode_heif_image(&source_path).unwrap();
    for extension in ["heic", "heif"] {
        for (index, rotation, mirror) in [
            (0, Some(1), None),
            (1, Some(3), None),
            (2, None, Some(0)),
            (3, Some(1), Some(1)),
        ] {
            let path = dir.join(format!("case-{index}.{extension}"));
            fs::write(&path, transformed_heif(rotation, mirror)).unwrap();
            let mut expected = match rotation {
                Some(1) => baseline.rotate270(),
                Some(3) => baseline.rotate90(),
                _ => baseline.clone(),
            };
            expected = match mirror {
                Some(0) => expected.fliph(),
                Some(1) => expected.flipv(),
                _ => expected,
            };
            let entry =
                inspect_single(&path, &dir, path.file_name().unwrap().to_str().unwrap()).unwrap();
            assert_eq!(
                (entry.width, entry.height),
                (Some(expected.width()), Some(expected.height()))
            );
            for mode in [
                MetadataMode::Strip,
                MetadataMode::DateOnly,
                MetadataMode::Keep,
            ] {
                let mut config = settings();
                config.metadata_mode = mode;
                let result = process_one(&entry, &config, &dir.join("output")).unwrap();
                assert_eq!(
                    image::open(result.output_path.unwrap()).unwrap().to_rgba8(),
                    expected.to_rgba8()
                );
            }
        }
    }
}

#[test]
fn source_copy_reports_display_dimensions_without_changing_orientation() {
    let dir = test_dir("copy");
    let path = dir.join("source.jpg");
    let source = DynamicImage::ImageRgba8(RgbaImage::from_fn(60, 40, |x, y| {
        let v = ((x * 71 + y * 37) % 256) as u8;
        Rgba([v, v, v, 255])
    }));
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 1)
        .encode_image(&source)
        .unwrap();
    let bytes = exif::embed(&bytes, OutputFormat::Jpeg, &tiff(8, false), 60, 40).unwrap();
    fs::write(&path, &bytes).unwrap();
    let entry = inspect_single(&path, &dir, "source.jpg").unwrap();
    let mut config = settings();
    config.output_format = OutputFormat::Original;
    config.metadata_mode = MetadataMode::Keep;
    let result = process_one(&entry, &config, &dir.join("output")).unwrap();
    let output = PathBuf::from(result.output_path.unwrap());
    assert!(result
        .warnings
        .iter()
        .any(|w| w.contains("元ファイルをコピー")));
    assert_eq!(fs::read(&output).unwrap(), bytes);
    assert_eq!(display_image_dimensions(&output).unwrap(), (40, 60));
    assert_eq!((result.width, result.height), (Some(40), Some(60)));
}
