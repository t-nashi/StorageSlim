//! EXIF の抽出・絞り込み・埋め込み。
//!
//! image クレートのエンコーダは EXIF を書き出さない。再エンコードすると
//! 撮影日時や向きが失われるため、元ファイルから取り出した EXIF を
//! エンコード後のコンテナへ自分で埋め戻す。
//!
//! 内部で受け渡すのは常に TIFF 構造の EXIF ブロブ（JPEG の APP1 から
//! `Exif\0\0` を除いた部分）に揃える。コンテナごとの包み方の違いは
//! 抽出・埋め込みの両端で吸収する。
//!
//! 壊れた EXIF で処理全体を落とさないため、解析は失敗しても `None` を返す。
//! パニックしうるスライス演算は使わない。

use crate::{InputFormat, OutputFormat};

// --- タグ番号 ---------------------------------------------------------------

const TAG_IMAGE_WIDTH: u16 = 0x0100;
const TAG_IMAGE_LENGTH: u16 = 0x0101;
const TAG_ORIENTATION: u16 = 0x0112;
const TAG_DATETIME: u16 = 0x0132;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_EXIF_VERSION: u16 = 0x9000;
const TAG_DATETIME_ORIGINAL: u16 = 0x9003;
const TAG_DATETIME_DIGITIZED: u16 = 0x9004;
const TAG_OFFSET_TIME: u16 = 0x9010;
const TAG_OFFSET_TIME_ORIGINAL: u16 = 0x9011;
const TAG_OFFSET_TIME_DIGITIZED: u16 = 0x9012;
const TAG_SUBSEC_TIME: u16 = 0x9290;
const TAG_SUBSEC_TIME_ORIGINAL: u16 = 0x9291;
const TAG_SUBSEC_TIME_DIGITIZED: u16 = 0x9292;
const TAG_PIXEL_X_DIMENSION: u16 = 0xA002;
const TAG_PIXEL_Y_DIMENSION: u16 = 0xA003;

/// 「撮影日のみ保持」で IFD0 から残すタグ。
///
/// 向きを残すのは、デコード時に回転を適用していないため。ここで落とすと
/// 縦位置で撮った写真が横向きで表示される。
const KEEP_IFD0: [u16; 2] = [TAG_ORIENTATION, TAG_DATETIME];

/// 「撮影日のみ保持」で Exif IFD から残すタグ。
const KEEP_EXIF: [u16; 9] = [
    TAG_EXIF_VERSION,
    TAG_DATETIME_ORIGINAL,
    TAG_DATETIME_DIGITIZED,
    TAG_OFFSET_TIME,
    TAG_OFFSET_TIME_ORIGINAL,
    TAG_OFFSET_TIME_DIGITIZED,
    TAG_SUBSEC_TIME,
    TAG_SUBSEC_TIME_ORIGINAL,
    TAG_SUBSEC_TIME_DIGITIZED,
];

/// JPEG の APP1 セグメントに入れられる EXIF の上限。
/// セグメント長は 16bit で、長さフィールド 2 バイトと `Exif\0\0` 6 バイトを引く。
const JPEG_APP1_MAX_EXIF: usize = 65_535 - 2 - 6;

// --- バイトオーダー ---------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ByteOrder {
    Little,
    Big,
}

impl ByteOrder {
    fn u16(self, buf: &[u8], at: usize) -> Option<u16> {
        let bytes = buf.get(at..at + 2)?;
        let pair = [bytes[0], bytes[1]];
        Some(match self {
            ByteOrder::Little => u16::from_le_bytes(pair),
            ByteOrder::Big => u16::from_be_bytes(pair),
        })
    }

    fn u32(self, buf: &[u8], at: usize) -> Option<u32> {
        let bytes = buf.get(at..at + 4)?;
        let quad = [bytes[0], bytes[1], bytes[2], bytes[3]];
        Some(match self {
            ByteOrder::Little => u32::from_le_bytes(quad),
            ByteOrder::Big => u32::from_be_bytes(quad),
        })
    }

    fn write_u16(self, out: &mut Vec<u8>, value: u16) {
        match self {
            ByteOrder::Little => out.extend_from_slice(&value.to_le_bytes()),
            ByteOrder::Big => out.extend_from_slice(&value.to_be_bytes()),
        }
    }

    fn write_u32(self, out: &mut Vec<u8>, value: u32) {
        match self {
            ByteOrder::Little => out.extend_from_slice(&value.to_le_bytes()),
            ByteOrder::Big => out.extend_from_slice(&value.to_be_bytes()),
        }
    }
}

/// TIFF のフィールド型が 1 要素あたり何バイトか。未知の型は扱わない。
fn type_size(field_type: u16) -> Option<usize> {
    Some(match field_type {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => return None,
    })
}

// --- 抽出 -------------------------------------------------------------------

/// 入力ファイルのバイト列から EXIF（TIFF ブロブ）を取り出す。
///
/// HEIC / HEIF は同梱デコーダが EXIF を露出しないため対象外。
pub(crate) fn extract(bytes: &[u8], format: &InputFormat) -> Option<Vec<u8>> {
    match format {
        InputFormat::Jpeg => extract_from_jpeg(bytes),
        InputFormat::Png => extract_from_png(bytes),
        InputFormat::Webp => extract_from_webp(bytes),
        _ => None,
    }
}

fn extract_from_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2usize;
    loop {
        // マーカーは 0xFF + 種別。パディングの 0xFF が連続することがある。
        if *bytes.get(pos)? != 0xFF {
            return None;
        }
        let mut marker_pos = pos;
        while *bytes.get(marker_pos)? == 0xFF {
            marker_pos += 1;
        }
        let marker = *bytes.get(marker_pos)?;
        // SOS 以降は圧縮データ。APP セグメントはそれより前にしか現れない。
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        let length = usize::from(ByteOrder::Big.u16(bytes, marker_pos + 1)?);
        if length < 2 {
            return None;
        }
        let payload = bytes.get(marker_pos + 3..marker_pos + 1 + length)?;
        if marker == 0xE1 && payload.starts_with(b"Exif\0\0") {
            return Some(payload.get(6..)?.to_vec());
        }
        pos = marker_pos + 1 + length;
    }
}

fn extract_from_png(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(0..8)? != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }
    let mut pos = 8usize;
    loop {
        let length = ByteOrder::Big.u32(bytes, pos)? as usize;
        let kind = bytes.get(pos + 4..pos + 8)?;
        if kind == b"IEND" {
            return None;
        }
        if kind == b"eXIf" {
            return Some(bytes.get(pos + 8..pos + 8 + length)?.to_vec());
        }
        // 長さ 4 + 種別 4 + データ + CRC 4
        pos = pos.checked_add(12)?.checked_add(length)?;
    }
}

fn extract_from_webp(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    for (kind, payload) in RiffChunks::new(bytes) {
        if kind == *b"EXIF" {
            return Some(payload.to_vec());
        }
    }
    None
}

/// RIFF のチャンクを順に返す。奇数長のチャンクは 1 バイトのパディングを伴う。
struct RiffChunks<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> RiffChunks<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 12 }
    }
}

impl<'a> Iterator for RiffChunks<'a> {
    type Item = ([u8; 4], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let kind = self.bytes.get(self.pos..self.pos + 4)?;
        let size = ByteOrder::Little.u32(self.bytes, self.pos + 4)? as usize;
        let payload = self.bytes.get(self.pos + 8..self.pos + 8 + size)?;
        let mut tag = [0u8; 4];
        tag.copy_from_slice(kind);
        self.pos = self.pos.checked_add(8)?.checked_add(size + (size & 1))?;
        Some((tag, payload))
    }
}

// --- 絞り込み ---------------------------------------------------------------

/// TIFF ヘッダを読み、バイトオーダーと IFD0 の位置を返す。
fn read_header(tiff: &[u8]) -> Option<(ByteOrder, usize)> {
    let order = match tiff.get(0..2)? {
        b"II" => ByteOrder::Little,
        b"MM" => ByteOrder::Big,
        _ => return None,
    };
    if order.u16(tiff, 2)? != 42 {
        return None;
    }
    Some((order, order.u32(tiff, 4)? as usize))
}

/// IFD の 1 エントリ。値は元のバイト列のまま持ち回る。
struct Entry<'a> {
    tag: u16,
    field_type: u16,
    count: u32,
    /// 4 バイトに収まる値。エントリの値フィールドをそのまま持つ。
    inline: [u8; 4],
    /// 4 バイトを超える値の実体。
    data: Option<&'a [u8]>,
}

fn read_ifd<'a>(tiff: &'a [u8], order: ByteOrder, at: usize) -> Option<Vec<Entry<'a>>> {
    let count = order.u16(tiff, at)? as usize;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let base = at.checked_add(2)?.checked_add(index.checked_mul(12)?)?;
        let tag = order.u16(tiff, base)?;
        let field_type = order.u16(tiff, base + 2)?;
        let value_count = order.u32(tiff, base + 4)?;
        let value_field = tiff.get(base + 8..base + 12)?;
        let mut inline = [0u8; 4];
        inline.copy_from_slice(value_field);

        // 型が未知のエントリは値の長さを決められないため捨てる。
        let Some(unit) = type_size(field_type) else {
            continue;
        };
        let total = unit.checked_mul(value_count as usize)?;
        let data = if total > 4 {
            let offset = order.u32(tiff, base + 8)? as usize;
            match tiff.get(offset..offset.checked_add(total)?) {
                Some(slice) => Some(slice),
                // 参照先が壊れているエントリは落とす。
                None => continue,
            }
        } else {
            None
        };

        entries.push(Entry {
            tag,
            field_type,
            count: value_count,
            inline,
            data,
        });
    }
    Some(entries)
}

/// 日付と向きだけを残した EXIF を組み直す。GPS IFD やメーカーノートは
/// 構造ごと落ちる。
///
/// 出力のバイトオーダーは入力に合わせる。値のバイト列をそのまま運べるため、
/// 型ごとのエンディアン変換を書かずに済む。
pub(crate) fn keep_date_and_orientation(tiff: &[u8]) -> Option<Vec<u8>> {
    let (order, ifd0_at) = read_header(tiff)?;
    let ifd0 = read_ifd(tiff, order, ifd0_at)?;

    let mut kept_ifd0: Vec<&Entry> = ifd0.iter().filter(|e| KEEP_IFD0.contains(&e.tag)).collect();
    kept_ifd0.sort_by_key(|e| e.tag);

    let exif_entries = ifd0
        .iter()
        .find(|e| e.tag == TAG_EXIF_IFD)
        .and_then(|e| order.u32(&e.inline, 0))
        .and_then(|at| read_ifd(tiff, order, at as usize))
        .unwrap_or_default();
    let mut kept_exif: Vec<&Entry> = exif_entries
        .iter()
        .filter(|e| KEEP_EXIF.contains(&e.tag))
        .collect();
    kept_exif.sort_by_key(|e| e.tag);

    if kept_ifd0.is_empty() && kept_exif.is_empty() {
        return None;
    }

    // Exif IFD を作る場合、IFD0 にはポインタのエントリが 1 つ増える。
    let ifd0_count = kept_ifd0.len() + usize::from(!kept_exif.is_empty());
    let ifd0_size = 2 + ifd0_count * 12 + 4;
    let exif_ifd_at = 8 + ifd0_size;
    let exif_ifd_size = if kept_exif.is_empty() {
        0
    } else {
        2 + kept_exif.len() * 12 + 4
    };
    let mut data_at = exif_ifd_at + exif_ifd_size;

    let mut out = Vec::new();
    match order {
        ByteOrder::Little => out.extend_from_slice(b"II"),
        ByteOrder::Big => out.extend_from_slice(b"MM"),
    }
    order.write_u16(&mut out, 42);
    order.write_u32(&mut out, 8);

    let mut heap = Vec::new();
    let write_ifd = |out: &mut Vec<u8>,
                         heap: &mut Vec<u8>,
                         data_at: &mut usize,
                         entries: &[&Entry],
                         pointer: Option<u32>| {
        let total = entries.len() + usize::from(pointer.is_some());
        order.write_u16(out, total as u16);
        for entry in entries {
            order.write_u16(out, entry.tag);
            order.write_u16(out, entry.field_type);
            order.write_u32(out, entry.count);
            match entry.data {
                Some(bytes) => {
                    order.write_u32(out, *data_at as u32);
                    heap.extend_from_slice(bytes);
                    // TIFF のオフセットは偶数境界に置く慣習に従う。
                    if bytes.len() % 2 == 1 {
                        heap.push(0);
                    }
                    *data_at += bytes.len() + bytes.len() % 2;
                }
                None => out.extend_from_slice(&entry.inline),
            }
        }
        if let Some(offset) = pointer {
            order.write_u16(out, TAG_EXIF_IFD);
            order.write_u16(out, 4);
            order.write_u32(out, 1);
            order.write_u32(out, offset);
        }
        // 次の IFD は無い。サムネイル (IFD1) は運ばない。
        order.write_u32(out, 0);
    };

    let pointer = (!kept_exif.is_empty()).then_some(exif_ifd_at as u32);
    write_ifd(&mut out, &mut heap, &mut data_at, &kept_ifd0, pointer);
    if !kept_exif.is_empty() {
        write_ifd(&mut out, &mut heap, &mut data_at, &kept_exif, None);
    }
    out.extend_from_slice(&heap);

    Some(out)
}

/// EXIF が持つ画素数のタグを、実際の出力寸法へ書き換える。
///
/// リサイズすると元の寸法が残ってしまうため。値が 4 バイトに収まる
/// エントリだけを対象に、既存のバイト列を上書きする。オフセットが動かないので
/// 他のエントリを壊さない。
pub(crate) fn patch_dimensions(tiff: &mut [u8], width: u32, height: u32) {
    let Some((order, ifd0_at)) = read_header(tiff) else {
        return;
    };
    let exif_ifd_at = read_ifd(tiff, order, ifd0_at)
        .unwrap_or_default()
        .iter()
        .find(|e| e.tag == TAG_EXIF_IFD)
        .and_then(|e| order.u32(&e.inline, 0))
        .map(|at| at as usize);

    for (at, tags) in [
        (Some(ifd0_at), [TAG_IMAGE_WIDTH, TAG_IMAGE_LENGTH]),
        (exif_ifd_at, [TAG_PIXEL_X_DIMENSION, TAG_PIXEL_Y_DIMENSION]),
    ] {
        let Some(at) = at else { continue };
        let Some(count) = order.u16(tiff, at) else {
            continue;
        };
        for index in 0..usize::from(count) {
            let base = at + 2 + index * 12;
            let (Some(tag), Some(field_type), Some(value_count)) = (
                order.u16(tiff, base),
                order.u16(tiff, base + 2),
                order.u32(tiff, base + 4),
            ) else {
                continue;
            };
            if value_count != 1 {
                continue;
            }
            let value = if tag == tags[0] {
                width
            } else if tag == tags[1] {
                height
            } else {
                continue;
            };
            let mut encoded = Vec::with_capacity(4);
            match field_type {
                // SHORT。上限を超える寸法は表現できないので触らない。
                3 if value <= u32::from(u16::MAX) => {
                    order.write_u16(&mut encoded, value as u16);
                    encoded.extend_from_slice(&[0, 0]);
                }
                4 => order.write_u32(&mut encoded, value),
                _ => continue,
            }
            if let Some(slot) = tiff.get_mut(base + 8..base + 12) {
                slot.copy_from_slice(&encoded);
            }
        }
    }
}

// --- 埋め込み ---------------------------------------------------------------

/// エンコード済みのコンテナへ EXIF を埋め込む。
///
/// 埋め込めない形式では `None` を返す。呼び出し側が警告に落とす。
pub(crate) fn embed(
    container: &[u8],
    format: OutputFormat,
    tiff: &[u8],
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    match format {
        // オリジナル維持の実体は JPEG エンコード。
        OutputFormat::Original | OutputFormat::Jpeg => embed_into_jpeg(container, tiff),
        OutputFormat::Png => embed_into_png(container, tiff),
        OutputFormat::Webp => embed_into_webp(container, tiff, width, height),
        // AVIF は meta box への挿入が必要で、GIF に EXIF の格納場所は無い。
        OutputFormat::Avif | OutputFormat::Gif => None,
    }
}

fn embed_into_jpeg(container: &[u8], tiff: &[u8]) -> Option<Vec<u8>> {
    if tiff.len() > JPEG_APP1_MAX_EXIF {
        return None;
    }
    if container.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }
    // JFIF の APP0 は先頭に置く決まりなので、その後ろへ差し込む。
    let mut insert_at = 2usize;
    if container.get(2..4) == Some(&[0xFF, 0xE0]) {
        let length = usize::from(ByteOrder::Big.u16(container, 4)?);
        insert_at = 4usize.checked_add(length)?;
        if insert_at > container.len() {
            return None;
        }
    }

    let mut out = Vec::with_capacity(container.len() + tiff.len() + 10);
    out.extend_from_slice(container.get(..insert_at)?);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(tiff);
    out.extend_from_slice(container.get(insert_at..)?);
    Some(out)
}

fn embed_into_png(container: &[u8], tiff: &[u8]) -> Option<Vec<u8>> {
    if container.get(0..8)? != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }
    // eXIf は IDAT より前に置く。
    let mut pos = 8usize;
    let insert_at = loop {
        let length = ByteOrder::Big.u32(container, pos)? as usize;
        let kind = container.get(pos + 4..pos + 8)?;
        if kind == b"IDAT" || kind == b"IEND" {
            break pos;
        }
        pos = pos.checked_add(12)?.checked_add(length)?;
    };

    let mut chunk = Vec::with_capacity(tiff.len() + 12);
    chunk.extend_from_slice(&(tiff.len() as u32).to_be_bytes());
    chunk.extend_from_slice(b"eXIf");
    chunk.extend_from_slice(tiff);
    let crc = crc32(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());

    let mut out = Vec::with_capacity(container.len() + chunk.len());
    out.extend_from_slice(container.get(..insert_at)?);
    out.extend_from_slice(&chunk);
    out.extend_from_slice(container.get(insert_at..)?);
    Some(out)
}

/// 単純形式の WebP は EXIF を持てない。VP8X を持つ拡張形式へ組み替えてから
/// EXIF チャンクを足す。既に VP8X があればフラグを立てるだけで済む。
fn embed_into_webp(container: &[u8], tiff: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if container.get(0..4)? != b"RIFF" || container.get(8..12)? != b"WEBP" {
        return None;
    }
    // VP8X の canvas は 24bit で「実寸 - 1」を格納する。
    if width == 0 || height == 0 || width > 1 << 24 || height > 1 << 24 {
        return None;
    }

    let mut body: Vec<u8> = Vec::with_capacity(container.len() + tiff.len() + 32);
    let mut has_vp8x = false;
    for (kind, payload) in RiffChunks::new(container) {
        if kind == *b"EXIF" {
            // 既存の EXIF は差し替える。
            continue;
        }
        let payload = if kind == *b"VP8X" {
            has_vp8x = true;
            let mut patched = payload.to_vec();
            // 先頭バイトのフラグに EXIF ビットを立てる。
            if let Some(flags) = patched.first_mut() {
                *flags |= 0x08;
            }
            patched
        } else {
            payload.to_vec()
        };
        write_riff_chunk(&mut body, &kind, &payload);
    }

    if !has_vp8x {
        let mut vp8x = Vec::with_capacity(10);
        vp8x.push(0x08);
        vp8x.extend_from_slice(&[0, 0, 0]);
        vp8x.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        vp8x.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        let mut wrapped = Vec::with_capacity(body.len() + 18);
        write_riff_chunk(&mut wrapped, b"VP8X", &vp8x);
        wrapped.extend_from_slice(&body);
        body = wrapped;
    }

    // EXIF は末尾に置く。
    write_riff_chunk(&mut body, b"EXIF", tiff);

    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    Some(out)
}

fn write_riff_chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
}

/// PNG チャンクの CRC32。この用途のためだけなので依存は足さない。
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub(crate) const DATETIME: &[u8; 20] = b"2019:05:03 11:22:33\0";
    pub(crate) const DATETIME_ORIGINAL: &[u8; 20] = b"2019:05:03 11:22:30\0";
    pub(crate) const MAKER_NOTE: &[u8; 8] = b"MAKERNOT";

    /// 撮影日時・向き・GPS・メーカーノートを含むリトルエンディアンの EXIF。
    ///
    /// 実ファイルから切り出すとテストがサンプル画像に依存するため、
    /// 構造を手で組み立てている。
    pub(crate) fn sample_tiff() -> Vec<u8> {
        const IFD0_AT: u32 = 8;
        const EXIF_IFD_AT: u32 = 62;
        const GPS_IFD_AT: u32 = 104;
        const DATETIME_AT: u32 = 122;
        const DATETIME_ORIGINAL_AT: u32 = 142;
        const MAKER_NOTE_AT: u32 = 162;

        fn entry(tag: u16, field_type: u16, count: u32, value: [u8; 4]) -> Vec<u8> {
            let mut out = Vec::with_capacity(12);
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&field_type.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&value);
            out
        }
        fn offset(at: u32) -> [u8; 4] {
            at.to_le_bytes()
        }
        fn short(value: u16) -> [u8; 4] {
            let bytes = value.to_le_bytes();
            [bytes[0], bytes[1], 0, 0]
        }

        let mut out = Vec::new();
        out.extend_from_slice(b"II");
        out.extend_from_slice(&42u16.to_le_bytes());
        out.extend_from_slice(&IFD0_AT.to_le_bytes());

        // IFD0
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend(entry(TAG_ORIENTATION, 3, 1, short(6)));
        out.extend(entry(TAG_DATETIME, 2, 20, offset(DATETIME_AT)));
        out.extend(entry(TAG_EXIF_IFD, 4, 1, offset(EXIF_IFD_AT)));
        out.extend(entry(0x8825, 4, 1, offset(GPS_IFD_AT)));
        out.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(out.len(), EXIF_IFD_AT as usize);

        // Exif IFD
        out.extend_from_slice(&3u16.to_le_bytes());
        out.extend(entry(
            TAG_DATETIME_ORIGINAL,
            2,
            20,
            offset(DATETIME_ORIGINAL_AT),
        ));
        out.extend(entry(0x927C, 7, 8, offset(MAKER_NOTE_AT)));
        out.extend(entry(TAG_PIXEL_X_DIMENSION, 4, 1, offset(4000)));
        out.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(out.len(), GPS_IFD_AT as usize);

        // GPS IFD
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend(entry(0x0001, 2, 2, [b'N', 0, 0, 0]));
        out.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(out.len(), DATETIME_AT as usize);

        out.extend_from_slice(DATETIME);
        out.extend_from_slice(DATETIME_ORIGINAL);
        out.extend_from_slice(MAKER_NOTE);
        out
    }

    pub(crate) fn find_ascii(tiff: &[u8], needle: &[u8]) -> bool {
        tiff.windows(needle.len()).any(|window| window == needle)
    }

}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn keeps_dates_and_orientation() {
        let filtered = keep_date_and_orientation(&sample_tiff()).expect("撮影日時が残るはず");
        assert!(find_ascii(&filtered, DATETIME));
        assert!(find_ascii(&filtered, DATETIME_ORIGINAL));

        let (order, ifd0_at) = read_header(&filtered).unwrap();
        let ifd0 = read_ifd(&filtered, order, ifd0_at).unwrap();
        let orientation = ifd0.iter().find(|e| e.tag == TAG_ORIENTATION).unwrap();
        assert_eq!(order.u16(&orientation.inline, 0), Some(6));
    }

    #[test]
    fn drops_gps_and_maker_note() {
        let filtered = keep_date_and_orientation(&sample_tiff()).expect("撮影日時が残るはず");
        assert!(!find_ascii(&filtered, MAKER_NOTE));

        let (order, ifd0_at) = read_header(&filtered).unwrap();
        let ifd0 = read_ifd(&filtered, order, ifd0_at).unwrap();
        assert!(ifd0.iter().all(|entry| entry.tag != 0x8825));
    }

    #[test]
    fn patches_pixel_dimensions() {
        let mut tiff = sample_tiff();
        patch_dimensions(&mut tiff, 1024, 768);

        let (order, ifd0_at) = read_header(&tiff).unwrap();
        let exif_at = read_ifd(&tiff, order, ifd0_at)
            .unwrap()
            .iter()
            .find(|entry| entry.tag == TAG_EXIF_IFD)
            .and_then(|entry| order.u32(&entry.inline, 0))
            .unwrap();
        let exif_ifd = read_ifd(&tiff, order, exif_at as usize).unwrap();
        let width = exif_ifd
            .iter()
            .find(|entry| entry.tag == TAG_PIXEL_X_DIMENSION)
            .unwrap();
        assert_eq!(order.u32(&width.inline, 0), Some(1024));
    }

    #[test]
    fn round_trips_through_jpeg() {
        let tiff = sample_tiff();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80)
            .encode(&[0u8; 4 * 4 * 3], 4, 4, image::ExtendedColorType::Rgb8)
            .unwrap();

        let embedded = embed(&jpeg, OutputFormat::Jpeg, &tiff, 4, 4).expect("埋め込めるはず");
        assert_eq!(extract(&embedded, &InputFormat::Jpeg).as_deref(), Some(&tiff[..]));
        // 画像として壊れていないこと。
        image::load_from_memory_with_format(&embedded, image::ImageFormat::Jpeg).unwrap();
    }

    #[test]
    fn round_trips_through_png() {
        use image::ImageEncoder;

        let tiff = sample_tiff();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[0u8; 4 * 4 * 4], 4, 4, image::ExtendedColorType::Rgba8)
            .unwrap();

        let embedded = embed(&png, OutputFormat::Png, &tiff, 4, 4).expect("埋め込めるはず");
        assert_eq!(extract(&embedded, &InputFormat::Png).as_deref(), Some(&tiff[..]));
        image::load_from_memory_with_format(&embedded, image::ImageFormat::Png).unwrap();
    }

    #[test]
    fn round_trips_through_webp() {
        let tiff = sample_tiff();
        let rgba = vec![255u8; 4 * 4 * 4];
        let webp = webp::Encoder::from_rgba(&rgba, 4, 4).encode(80.0).to_vec();

        let embedded = embed(&webp, OutputFormat::Webp, &tiff, 4, 4).expect("埋め込めるはず");
        assert_eq!(extract(&embedded, &InputFormat::Webp).as_deref(), Some(&tiff[..]));
        image::load_from_memory_with_format(&embedded, image::ImageFormat::WebP).unwrap();
    }

    #[test]
    fn rejects_formats_without_exif_storage() {
        let tiff = sample_tiff();
        assert!(embed(b"GIF89a", OutputFormat::Gif, &tiff, 4, 4).is_none());
        assert!(embed(&[0u8; 32], OutputFormat::Avif, &tiff, 4, 4).is_none());
    }

    #[test]
    fn survives_broken_input() {
        // 壊れた EXIF でバッチ全体を落とさないこと。
        assert!(keep_date_and_orientation(&[]).is_none());
        assert!(keep_date_and_orientation(b"II").is_none());
        assert!(keep_date_and_orientation(b"II\x2a\x00\xff\xff\xff\xff").is_none());
        let mut truncated = sample_tiff();
        truncated.truncate(70);
        let _ = keep_date_and_orientation(&truncated);
        let mut patched = truncated.clone();
        patch_dimensions(&mut patched, 10, 10);
    }
}
