#!/usr/bin/env python3
"""samples/input/ の PSD サンプルを生成する。

ImageMagick が書き出す PSD は統合画像を常に無圧縮で持つため、Photoshop が使う
RLE (PackBits) の経路がサンプルで踏めない。無圧縮で書き出したものを読み直し、
画像データセクションだけを RLE へ詰め直したファイルも併せて作る。

使い方:
    python3 scripts/generate-psd-samples.py
"""

import struct
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INPUT_DIR = ROOT / "samples" / "input"
SOURCE = INPUT_DIR / "sample-photo.jpg"
SIZE = "160x90"


def run_magick(args, output):
    subprocess.run(["magick", str(SOURCE), "-resize", SIZE, *args, str(output)], check=True)


def packbits(data: bytes) -> bytes:
    """PackBits で 1 行分を圧縮する。"""
    out = bytearray()
    i = 0
    n = len(data)
    while i < n:
        run = 1
        while i + run < n and data[i + run] == data[i] and run < 128:
            run += 1
        if run >= 2:
            # 繰り返し: 1-n 回として負の長さで表す。
            out.append(256 - (run - 1))
            out.append(data[i])
            i += run
            continue
        # リテラル: 次に 3 連続が現れるまでをそのまま並べる。
        start = i
        literal = 0
        while i < n and literal < 128:
            if i + 2 < n and data[i] == data[i + 1] == data[i + 2]:
                break
            i += 1
            literal += 1
        out.append(literal - 1)
        out += data[start : start + literal]
    return bytes(out)


def to_rle(source: Path, destination: Path):
    """無圧縮の PSD を読み、画像データセクションを RLE へ詰め直す。"""
    raw = source.read_bytes()

    channels = struct.unpack(">H", raw[12:14])[0]
    height = struct.unpack(">I", raw[14:18])[0]
    width = struct.unpack(">I", raw[18:22])[0]

    # カラーモードデータ / 画像リソース / レイヤー & マスク情報を読み飛ばす。
    pos = 26
    for _ in range(3):
        length = struct.unpack(">I", raw[pos : pos + 4])[0]
        pos += 4 + length

    prefix = raw[:pos]
    compression = struct.unpack(">H", raw[pos : pos + 2])[0]
    if compression != 0:
        raise SystemExit(f"想定外の圧縮方式です: {compression}")

    body = raw[pos + 2 :]
    expected = channels * height * width
    if len(body) != expected:
        raise SystemExit(f"画像データの長さが合いません: {len(body)} != {expected}")

    counts = []
    rows = bytearray()
    for channel in range(channels):
        for y in range(height):
            offset = (channel * height + y) * width
            packed = packbits(body[offset : offset + width])
            counts.append(len(packed))
            rows += packed

    out = bytearray(prefix)
    out += struct.pack(">H", 1)
    for count in counts:
        out += struct.pack(">H", count)
    out += rows
    destination.write_bytes(bytes(out))


def main():
    run_magick(["-colorspace", "sRGB"], INPUT_DIR / "sample-psd.psd")
    run_magick(["-colorspace", "CMYK"], INPUT_DIR / "sample-psd-cmyk.psd")
    run_magick(["-depth", "16", "-colorspace", "sRGB"], INPUT_DIR / "sample-psd-16bit.psd")
    to_rle(INPUT_DIR / "sample-psd.psd", INPUT_DIR / "sample-psd-rle.psd")


if __name__ == "__main__":
    main()
