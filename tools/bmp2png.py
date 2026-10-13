import struct
import sys
import zlib


def read_bmp(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:2] != b"BM" or struct.unpack_from("<I", data, 30)[0] != 0:
        raise SystemExit("not an uncompressed BMP")
    offset = struct.unpack_from("<I", data, 10)[0]
    width, height = struct.unpack_from("<ii", data, 18)
    bpp = struct.unpack_from("<H", data, 28)[0]
    if bpp != 32:
        raise SystemExit("expected 32bpp")
    bottom_up = height > 0
    height = abs(height)
    rows = []
    stride = width * 4
    for y in range(height):
        src = offset + y * stride
        row = data[src : src + stride]
        rows.append(row if bottom_up else row)
    if bottom_up:
        rows.reverse()
    pixels = bytearray()
    for row in rows:
        for i in range(0, stride, 4):
            b, g, r, a = row[i : i + 4]
            pixels += bytes((r, g, b, a))
    return width, height, pixels


def write_png(path, width, height, rgba):
    def chunk(tag, payload):
        return (
            struct.pack(">I", len(payload))
            + tag
            + payload
            + struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF)
        )

    raw = b"".join(
        b"\x00" + bytes(rgba[y * width * 4 : (y + 1) * width * 4]) for y in range(height)
    )
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", ihdr))
        f.write(chunk(b"IDAT", zlib.compress(raw, 6)))
        f.write(chunk(b"IEND", b""))


if __name__ == "__main__":
    w, h, px = read_bmp(sys.argv[1])
    write_png(sys.argv[2], w, h, px)
    print(f"ok {w}x{h}")
