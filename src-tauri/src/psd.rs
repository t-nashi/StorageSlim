//! PSD (Adobe Photoshop Document) から統合画像 (composite) だけを取り出す。
//!
//! PSD は編集用の形式で、レイヤーデータがファイルの大半を占める。このアプリが必要と
//! するのは Photoshop が書き出した統合後の 1 枚だけなので、レイヤーセクションは
//! 長さ分をシークで読み飛ばし、末尾の画像データセクションだけを読む。巨大な PSD でも
//! レイヤー分のメモリを確保せずに済む。
//!
//! 対応範囲は意図的に狭くしてある:
//!
//! - PSD のみ (PSB は非対応)
//! - カラーモード RGB のみ (CMYK / Lab などは ICC を持たないと色が変わるため扱わない)
//! - 8bit/channel のみ
//! - 統合画像を持つファイルのみ (Photoshop の「互換性を優先」で保存されたもの)
//!
//! 範囲外のファイルは [`PsdProbe::unsupported_reason`] が理由を返す。呼び出し側は
//! これを入力一覧の警告として見せ、実行前にユーザーへ知らせる。

use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{anyhow, Context, Result};
use image::{DynamicImage, RgbaImage};

/// ファイル先頭のシグネチャ。
const SIGNATURE: &[u8; 4] = b"8BPS";
/// 画像リソースブロックのシグネチャ。
const RESOURCE_SIGNATURE: &[u8; 4] = b"8BIM";
/// 画像リソース ID: Version Info。統合画像を実際に持っているかのフラグを含む。
const RESOURCE_ID_VERSION_INFO: u16 = 1057;
/// 画像リソース ID: EXIF データ (生の TIFF ブロック)。
const RESOURCE_ID_EXIF: u16 = 1058;

/// PSD のカラーモード。ヘッダの数値をそのまま持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorMode {
    Bitmap,
    Grayscale,
    Indexed,
    Rgb,
    Cmyk,
    Multichannel,
    Duotone,
    Lab,
    Unknown(u16),
}

impl ColorMode {
    fn from_raw(value: u16) -> Self {
        match value {
            0 => ColorMode::Bitmap,
            1 => ColorMode::Grayscale,
            2 => ColorMode::Indexed,
            3 => ColorMode::Rgb,
            4 => ColorMode::Cmyk,
            7 => ColorMode::Multichannel,
            8 => ColorMode::Duotone,
            9 => ColorMode::Lab,
            other => ColorMode::Unknown(other),
        }
    }

    fn label(self) -> &'static str {
        match self {
            ColorMode::Bitmap => "モノクロ 2 階調",
            ColorMode::Grayscale => "グレースケール",
            ColorMode::Indexed => "インデックスカラー",
            ColorMode::Rgb => "RGB",
            ColorMode::Cmyk => "CMYK",
            ColorMode::Multichannel => "マルチチャンネル",
            ColorMode::Duotone => "ダブルトーン",
            ColorMode::Lab => "Lab",
            ColorMode::Unknown(_) => "不明なカラーモード",
        }
    }
}

/// デコードせずに読み取れる PSD の素性。
#[derive(Debug, Clone)]
pub(crate) struct PsdProbe {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) channels: u16,
    pub(crate) depth: u16,
    pub(crate) color_mode: ColorMode,
    /// PSB (Large Document Format) かどうか。
    pub(crate) is_psb: bool,
    /// 統合画像を持っているか。「互換性を優先」を外して保存されたファイルは false。
    pub(crate) has_composite: bool,
}

impl PsdProbe {
    /// このアプリで扱えない理由。扱えるなら `None`。
    ///
    /// 文言はそのまま入力一覧の警告として出るため、ユーザーが次に取るべき操作まで書く。
    pub(crate) fn unsupported_reason(&self) -> Option<String> {
        if self.is_psb {
            return Some(
                "PSB (ラージドキュメント) は非対応です。PSD として保存し直してください。"
                    .to_string(),
            );
        }
        if self.color_mode != ColorMode::Rgb {
            return Some(format!(
                "{} の PSD は非対応です。Photoshop で RGB カラーへ変換してから保存してください。",
                self.color_mode.label()
            ));
        }
        if self.depth != 8 {
            return Some(format!(
                "{}bit/チャンネルの PSD は非対応です。8bit/チャンネルへ変換してから保存してください。",
                self.depth
            ));
        }
        if self.channels < 3 {
            return Some("チャンネル数が不足しており統合画像を読めません。".to_string());
        }
        if !self.has_composite {
            return Some(NO_COMPOSITE_MESSAGE.to_string());
        }
        None
    }
}

/// 統合画像が無いときの案内。probe 時と decode 時の双方から使うため定数にしてある。
const NO_COMPOSITE_MESSAGE: &str =
    "統合画像を持たない PSD です。Photoshop の保存時に「互換性を優先」を有効にして保存し直してください。";

/// ヘッダと画像リソースだけを読み、デコードはしない。
pub(crate) fn probe(path: &Path) -> Result<PsdProbe> {
    let file = File::open(path).with_context(|| format!("failed to open PSD: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let header = read_header(&mut reader)
        .with_context(|| format!("failed to read PSD header: {}", path.display()))?;

    // PSB はレイヤーセクションの長さフィールド幅が違う。読み進めると位置がずれるため、
    // ヘッダの情報だけを返してここで止める。
    if header.is_psb {
        return Ok(PsdProbe {
            width: header.width,
            height: header.height,
            channels: header.channels,
            depth: header.depth,
            color_mode: header.color_mode,
            is_psb: true,
            has_composite: false,
        });
    }

    skip_length_prefixed_section(&mut reader).context("failed to skip color mode data")?;
    let resources = read_image_resources(&mut reader).context("failed to read image resources")?;

    // Version Info が無い PSD (Photoshop 以外が書いたものなど) は、フラグでは判断
    // できないので持っている前提で進め、デコード時に失敗させる。
    let has_composite = resources.has_real_merged_data.unwrap_or(true);

    Ok(PsdProbe {
        width: header.width,
        height: header.height,
        channels: header.channels,
        depth: header.depth,
        color_mode: header.color_mode,
        is_psb: false,
        has_composite,
    })
}

/// PSD に埋め込まれた EXIF (生の TIFF ブロック) を取り出す。無ければ `None`。
pub(crate) fn read_exif(path: &Path) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let header = read_header(&mut reader).ok()?;
    if header.is_psb {
        return None;
    }
    skip_length_prefixed_section(&mut reader).ok()?;
    read_image_resources(&mut reader).ok()?.exif
}

/// 統合画像を RGBA8 へ展開する。
///
/// `decode_limit_bytes` はピクセルバッファに確保を許すバイト数。`image` クレートを
/// 通らないため、上限の判定もここで行う。
pub(crate) fn decode_composite(path: &Path, decode_limit_bytes: u64) -> Result<DynamicImage> {
    let file = File::open(path).with_context(|| format!("failed to open PSD: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let header = read_header(&mut reader)
        .with_context(|| format!("failed to read PSD header: {}", path.display()))?;

    if header.is_psb {
        return Err(anyhow!("PSB (ラージドキュメント) は非対応です。"));
    }
    if header.color_mode != ColorMode::Rgb {
        return Err(anyhow!(
            "{} の PSD は非対応です。",
            header.color_mode.label()
        ));
    }
    if header.depth != 8 {
        return Err(anyhow!("{}bit/チャンネルの PSD は非対応です。", header.depth));
    }
    if header.channels < 3 {
        return Err(anyhow!("チャンネル数が不足しており統合画像を読めません。"));
    }
    if header.width == 0 || header.height == 0 {
        return Err(anyhow!("寸法が 0 の PSD です。"));
    }

    let required = u64::from(header.width) * u64::from(header.height) * 4;
    if required > decode_limit_bytes {
        return Err(anyhow!(
            "デコードに約 {} MB が必要で、上限 {} MB を超えています。デコード上限を上げてください。",
            required / (1024 * 1024),
            decode_limit_bytes / (1024 * 1024)
        ));
    }

    skip_length_prefixed_section(&mut reader).context("failed to skip color mode data")?;
    skip_length_prefixed_section(&mut reader).context("failed to skip image resources")?;
    skip_length_prefixed_section(&mut reader).context("failed to skip layer and mask info")?;

    let compression = read_u16(&mut reader).context("failed to read compression method")?;
    let width = header.width as usize;
    let height = header.height as usize;
    // アルファを持たない PSD もあるため、先に不透明で埋めておく。
    let mut pixels = vec![255u8; width * height * 4];
    // R / G / B / A の 4 つが取れれば十分。5 番目以降はスポットチャンネルなので読まない。
    let used_channels = header.channels.min(4) as usize;

    match compression {
        0 => read_raw_channels(&mut reader, &mut pixels, width, height, used_channels)?,
        1 => read_rle_channels(
            &mut reader,
            &mut pixels,
            width,
            height,
            used_channels,
            header.channels as usize,
        )?,
        2 | 3 => {
            return Err(anyhow!(
                "ZIP 圧縮された統合画像には対応していません。Photoshop で保存し直してください。"
            ))
        }
        other => return Err(anyhow!("未知の圧縮方式です (compression = {other})。")),
    }

    let image = RgbaImage::from_raw(header.width, header.height, pixels)
        .ok_or_else(|| anyhow!("統合画像の展開結果が寸法と一致しません。"))?;
    Ok(DynamicImage::ImageRgba8(image))
}

struct Header {
    width: u32,
    height: u32,
    channels: u16,
    depth: u16,
    color_mode: ColorMode,
    is_psb: bool,
}

/// ファイルヘッダ (26 byte) を読む。
fn read_header<R: Read>(reader: &mut R) -> Result<Header> {
    let mut buffer = [0u8; 26];
    reader.read_exact(&mut buffer)?;

    if &buffer[0..4] != SIGNATURE {
        return Err(anyhow!("PSD のシグネチャではありません。"));
    }

    let version = u16::from_be_bytes([buffer[4], buffer[5]]);
    // buffer[6..12] は予約領域 (常に 0)。
    let channels = u16::from_be_bytes([buffer[12], buffer[13]]);
    let height = u32::from_be_bytes([buffer[14], buffer[15], buffer[16], buffer[17]]);
    let width = u32::from_be_bytes([buffer[18], buffer[19], buffer[20], buffer[21]]);
    let depth = u16::from_be_bytes([buffer[22], buffer[23]]);
    let color_mode = ColorMode::from_raw(u16::from_be_bytes([buffer[24], buffer[25]]));

    if version != 1 && version != 2 {
        return Err(anyhow!("未知の PSD バージョンです (version = {version})。"));
    }

    Ok(Header {
        width,
        height,
        channels,
        depth,
        color_mode,
        is_psb: version == 2,
    })
}

/// PSD の数値はすべてビッグエンディアン。
fn read_u16<R: Read>(reader: &mut R) -> Result<u16> {
    let mut buffer = [0u8; 2];
    reader.read_exact(&mut buffer)?;
    Ok(u16::from_be_bytes(buffer))
}

fn read_u32<R: Read>(reader: &mut R) -> Result<u32> {
    let mut buffer = [0u8; 4];
    reader.read_exact(&mut buffer)?;
    Ok(u32::from_be_bytes(buffer))
}

/// u32 の長さが前置されたセクションを読み飛ばす。
fn skip_length_prefixed_section<R: Read + Seek>(reader: &mut R) -> Result<()> {
    let length = read_u32(reader)?;
    reader.seek(SeekFrom::Current(i64::from(length)))?;
    Ok(())
}

#[derive(Default)]
struct ImageResources {
    /// Version Info (1057) の `hasRealMergedData`。リソース自体が無ければ `None`。
    has_real_merged_data: Option<bool>,
    /// EXIF (1058) の生 TIFF ブロック。
    exif: Option<Vec<u8>>,
}

/// 画像リソースセクションを走査し、必要なブロックだけを拾う。
///
/// 各ブロックは `8BIM` + ID(u16) + Pascal 文字列の名前 + 長さ(u32) + データ、という
/// 並びで、名前とデータはそれぞれ偶数バイトへパディングされる。
fn read_image_resources<R: Read + Seek>(reader: &mut R) -> Result<ImageResources> {
    let section_length = u64::from(read_u32(reader)?);
    let section_start = reader.stream_position()?;
    let section_end = section_start + section_length;
    let mut found = ImageResources::default();

    while reader.stream_position()? + 12 <= section_end {
        let mut signature = [0u8; 4];
        reader.read_exact(&mut signature)?;
        if &signature != RESOURCE_SIGNATURE {
            // 並びが壊れている。ここから先は信用できないので走査をやめる。
            break;
        }

        let id = read_u16(reader)?;

        // Pascal 文字列: 先頭 1 byte が長さ。長さ込みで偶数になるようパディングされる。
        let mut name_length = [0u8; 1];
        reader.read_exact(&mut name_length)?;
        let name_bytes = usize::from(name_length[0]);
        let name_padding = if (name_bytes + 1) % 2 == 0 { 0 } else { 1 };
        reader.seek(SeekFrom::Current((name_bytes + name_padding) as i64))?;

        let data_length = read_u32(reader)? as usize;
        let data_padding = data_length % 2;

        match id {
            RESOURCE_ID_VERSION_INFO if data_length >= 5 => {
                let mut head = [0u8; 5];
                reader.read_exact(&mut head)?;
                // 先頭 4 byte は version、続く 1 byte が hasRealMergedData。
                found.has_real_merged_data = Some(head[4] != 0);
                reader.seek(SeekFrom::Current((data_length - 5 + data_padding) as i64))?;
            }
            RESOURCE_ID_EXIF if data_length > 0 => {
                let mut data = vec![0u8; data_length];
                reader.read_exact(&mut data)?;
                found.exif = Some(data);
                reader.seek(SeekFrom::Current(data_padding as i64))?;
            }
            _ => {
                reader.seek(SeekFrom::Current((data_length + data_padding) as i64))?;
            }
        }
    }

    // 次のセクションから読み始められるよう、位置を必ずセクション末尾へ揃える。
    reader.seek(SeekFrom::Start(section_end))?;
    Ok(found)
}

/// 無圧縮のチャンネルデータを読む。
///
/// PSD はチャンネルごとに全画素を並べる (RRR...GGG...BBB...) ため、読みながら
/// RGBA へインターリーブする。中間バッファを持たないことで確保量を抑える。
fn read_raw_channels<R: Read>(
    reader: &mut R,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    used_channels: usize,
) -> Result<()> {
    let mut row = vec![0u8; width];
    for channel in 0..used_channels {
        for y in 0..height {
            reader
                .read_exact(&mut row)
                .with_context(|| format!("統合画像のデータが不足しています (channel {channel}, row {y})"))?;
            scatter_row(pixels, &row, width, y, channel);
        }
    }
    Ok(())
}

/// RLE (PackBits) 圧縮のチャンネルデータを読む。
///
/// 先頭に「全チャンネル × 全行」の圧縮後バイト数テーブルが並び、その後に実データが続く。
fn read_rle_channels<R: Read + Seek>(
    reader: &mut R,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    used_channels: usize,
    total_channels: usize,
) -> Result<()> {
    let mut byte_counts = vec![0u16; total_channels * height];
    for count in byte_counts.iter_mut() {
        *count = read_u16(reader).context("圧縮テーブルの読み込みに失敗しました")?;
    }

    let mut packed = Vec::new();
    let mut row = vec![0u8; width];
    for channel in 0..used_channels {
        for y in 0..height {
            let count = usize::from(byte_counts[channel * height + y]);
            packed.resize(count, 0);
            reader
                .read_exact(&mut packed)
                .with_context(|| format!("統合画像のデータが不足しています (channel {channel}, row {y})"))?;
            unpack_bits(&packed, &mut row)
                .with_context(|| format!("RLE の展開に失敗しました (channel {channel}, row {y})"))?;
            scatter_row(pixels, &row, width, y, channel);
        }
    }
    Ok(())
}

/// 1 行分のチャンネル値を RGBA バッファの該当成分へ書き込む。
fn scatter_row(pixels: &mut [u8], row: &[u8], width: usize, y: usize, channel: usize) {
    let base = y * width * 4 + channel;
    for (x, value) in row.iter().enumerate() {
        pixels[base + x * 4] = *value;
    }
}

/// PackBits を展開する。`output` は展開後の長さちょうどで渡す。
///
/// 先頭バイトを i8 として読み、0 以上なら「続く n+1 byte をそのままコピー」、
/// 負なら「次の 1 byte を 1-n 回繰り返す」。-128 は何もしない。
fn unpack_bits(input: &[u8], output: &mut [u8]) -> Result<()> {
    let mut read = 0usize;
    let mut written = 0usize;

    while written < output.len() {
        let header = *input
            .get(read)
            .ok_or_else(|| anyhow!("入力が途中で終わっています"))? as i8;
        read += 1;

        if header == -128 {
            continue;
        }

        if header >= 0 {
            let count = header as usize + 1;
            let end = read + count;
            let chunk = input
                .get(read..end)
                .ok_or_else(|| anyhow!("入力が途中で終わっています"))?;
            let limit = count.min(output.len() - written);
            output[written..written + limit].copy_from_slice(&chunk[..limit]);
            read = end;
            written += limit;
        } else {
            let count = (1 - i32::from(header)) as usize;
            let value = *input
                .get(read)
                .ok_or_else(|| anyhow!("入力が途中で終わっています"))?;
            read += 1;
            let limit = count.min(output.len() - written);
            output[written..written + limit].fill(value);
            written += limit;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn header_bytes(version: u16, channels: u16, depth: u16, color_mode: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(SIGNATURE);
        bytes.extend_from_slice(&version.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 6]);
        bytes.extend_from_slice(&channels.to_be_bytes());
        bytes.extend_from_slice(&2u32.to_be_bytes()); // height
        bytes.extend_from_slice(&3u32.to_be_bytes()); // width
        bytes.extend_from_slice(&depth.to_be_bytes());
        bytes.extend_from_slice(&color_mode.to_be_bytes());
        bytes
    }

    #[test]
    fn reads_header_fields() {
        let bytes = header_bytes(1, 4, 8, 3);
        let header = read_header(&mut Cursor::new(bytes)).unwrap();
        assert_eq!((header.width, header.height), (3, 2));
        assert_eq!(header.channels, 4);
        assert_eq!(header.depth, 8);
        assert_eq!(header.color_mode, ColorMode::Rgb);
        assert!(!header.is_psb);
    }

    #[test]
    fn rejects_foreign_signature() {
        let bytes = vec![b'8', b'B', b'I', b'M', 0, 1];
        assert!(read_header(&mut Cursor::new(bytes)).is_err());
    }

    #[test]
    fn detects_psb_version() {
        let header = read_header(&mut Cursor::new(header_bytes(2, 4, 8, 3))).unwrap();
        assert!(header.is_psb);
    }

    fn probe_of(width: u32, height: u32, channels: u16, depth: u16, mode: ColorMode, psb: bool, composite: bool) -> PsdProbe {
        PsdProbe {
            width,
            height,
            channels,
            depth,
            color_mode: mode,
            is_psb: psb,
            has_composite: composite,
        }
    }

    #[test]
    fn accepts_rgb_8bit_with_composite() {
        let probe = probe_of(10, 10, 4, 8, ColorMode::Rgb, false, true);
        assert!(probe.unsupported_reason().is_none());
    }

    #[test]
    fn rejects_cmyk_and_high_depth_and_psb() {
        assert!(probe_of(10, 10, 5, 8, ColorMode::Cmyk, false, true)
            .unsupported_reason()
            .unwrap()
            .contains("CMYK"));
        assert!(probe_of(10, 10, 4, 16, ColorMode::Rgb, false, true)
            .unsupported_reason()
            .unwrap()
            .contains("16bit"));
        assert!(probe_of(10, 10, 4, 8, ColorMode::Rgb, true, true)
            .unsupported_reason()
            .unwrap()
            .contains("PSB"));
    }

    #[test]
    fn reports_missing_composite() {
        let reason = probe_of(10, 10, 4, 8, ColorMode::Rgb, false, false)
            .unsupported_reason()
            .unwrap();
        assert!(reason.contains("互換性を優先"));
    }

    #[test]
    fn unpacks_literal_and_repeat_runs() {
        // 2 byte のリテラル (1, 2) と、3 の 4 回繰り返し。
        let input = vec![1, 1, 2, 0xFD, 3];
        let mut output = vec![0u8; 6];
        unpack_bits(&input, &mut output).unwrap();
        assert_eq!(output, vec![1, 2, 3, 3, 3, 3]);
    }

    #[test]
    fn unpack_skips_noop_marker() {
        let input = vec![0x80, 0, 9];
        let mut output = vec![0u8; 1];
        unpack_bits(&input, &mut output).unwrap();
        assert_eq!(output, vec![9]);
    }

    #[test]
    fn unpack_fails_on_truncated_input() {
        let input = vec![5, 1, 2];
        let mut output = vec![0u8; 6];
        assert!(unpack_bits(&input, &mut output).is_err());
    }

    /// 画像リソースブロック 1 個分。名前は空、データは偶数へパディングされる。
    fn resource_block(id: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(RESOURCE_SIGNATURE);
        bytes.extend_from_slice(&id.to_be_bytes());
        bytes.extend_from_slice(&[0u8, 0u8]); // 空の Pascal 文字列 + パディング
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend_from_slice(data);
        if data.len() % 2 == 1 {
            bytes.push(0);
        }
        bytes
    }

    /// 画像データセクションの手前までを組み立てた PSD を一時ファイルへ書く。
    fn write_psd_with_resources(name: &str, resources: Vec<u8>) -> std::path::PathBuf {
        let mut bytes = header_bytes(1, 3, 8, 3);
        bytes.extend_from_slice(&0u32.to_be_bytes()); // カラーモードデータ: 空
        bytes.extend_from_slice(&(resources.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&resources);
        bytes.extend_from_slice(&0u32.to_be_bytes()); // レイヤー & マスク情報: 空

        let path = std::env::temp_dir().join(format!("storageslim-psd-{}-{}", name, std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn reads_exif_resource() {
        let tiff = b"II\x2a\x00exif-body".to_vec();
        let mut resources = resource_block(RESOURCE_ID_VERSION_INFO, &[0, 0, 0, 1, 1]);
        resources.extend_from_slice(&resource_block(RESOURCE_ID_EXIF, &tiff));
        let path = write_psd_with_resources("exif", resources);

        assert_eq!(read_exif(&path), Some(tiff));
        assert!(probe(&path).unwrap().has_composite);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn detects_missing_merged_data_flag() {
        // hasRealMergedData = 0。「互換性を優先」を外して保存された PSD に相当する。
        let resources = resource_block(RESOURCE_ID_VERSION_INFO, &[0, 0, 0, 1, 0]);
        let path = write_psd_with_resources("no-merged", resources);

        let probe = probe(&path).unwrap();
        assert!(!probe.has_composite);
        assert!(probe.unsupported_reason().unwrap().contains("互換性を優先"));
        assert_eq!(read_exif(&path), None);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn scatters_channel_rows_into_rgba() {
        let mut pixels = vec![255u8; 3 * 2 * 4];
        scatter_row(&mut pixels, &[1, 2, 3], 3, 0, 0);
        scatter_row(&mut pixels, &[4, 5, 6], 3, 1, 2);
        assert_eq!(pixels[0], 1);
        assert_eq!(pixels[4], 2);
        assert_eq!(pixels[8], 3);
        assert_eq!(pixels[3 * 4 + 2], 4);
        assert_eq!(pixels[4 * 4 + 2], 5);
    }
}

