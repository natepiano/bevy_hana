"""Generate a well-formed 128-byte EDID for a 1920x1080 low-DPI (~92 DPI) monitor.

Large physical size (520x290 mm) at 1920x1080 => ~92 DPI, so Windows should recommend
100% scale (scale factor 1), unlike the VDD's built-in 800x600 EDID that lands at 200%.
Keeps manufacturer "MTT" + product 0x1337 so the DISPLAY\\MTT1337 identity (and the
harness target matcher) is preserved.
"""

from __future__ import annotations

import sys

H_RES = 1920
V_RES = 1080
H_MM = 520
V_MM = 290


def detailed_timing() -> bytes:
    # 1920x1080 @ 60 Hz, CEA-861 / VESA standard timing.
    pixel_clock_10khz = 14850  # 148.50 MHz
    h_active, h_blank = 1920, 280
    v_active, v_blank = 1080, 45
    h_sync_off, h_sync_w = 88, 44
    v_sync_off, v_sync_w = 4, 5
    b = bytearray(18)
    b[0] = pixel_clock_10khz & 0xFF
    b[1] = (pixel_clock_10khz >> 8) & 0xFF
    b[2] = h_active & 0xFF
    b[3] = h_blank & 0xFF
    b[4] = ((h_active >> 8) << 4) | (h_blank >> 8)
    b[5] = v_active & 0xFF
    b[6] = v_blank & 0xFF
    b[7] = ((v_active >> 8) << 4) | (v_blank >> 8)
    b[8] = h_sync_off & 0xFF
    b[9] = h_sync_w & 0xFF
    b[10] = ((v_sync_off & 0x0F) << 4) | (v_sync_w & 0x0F)
    b[11] = (
        ((h_sync_off >> 8) << 6)
        | ((h_sync_w >> 8) << 4)
        | ((v_sync_off >> 4) << 2)
        | (v_sync_w >> 4)
    )
    b[12] = H_MM & 0xFF
    b[13] = V_MM & 0xFF
    b[14] = ((H_MM >> 8) << 4) | (V_MM >> 8)
    b[15] = 0  # h border
    b[16] = 0  # v border
    b[17] = 0x1E  # digital, separate sync, +/+
    return bytes(b)


def text_descriptor(tag: int, text: str) -> bytes:
    body = text.encode("ascii")[:13].ljust(13, b"\x20")
    return bytes([0x00, 0x00, 0x00, tag, 0x00]) + body


def range_descriptor() -> bytes:
    # Monitor range limits: 50-75 Hz V, 30-83 kHz H, max pixel clock 170 MHz.
    return bytes([0x00, 0x00, 0x00, 0xFD, 0x00, 50, 75, 30, 83, 17, 0x00, 0x0A]) + b"\x20" * 6


def build() -> bytes:
    e = bytearray(128)
    e[0:8] = bytes([0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00])
    # Manufacturer "MTT": M=13,T=20,T=20 -> (13<<10)|(20<<5)|20 = 0x3694
    e[8] = 0x36
    e[9] = 0x94
    # Product 0x1337, little-endian
    e[10] = 0x37
    e[11] = 0x13
    # Serial number
    e[12:16] = bytes([0x01, 0x00, 0x00, 0x00])
    e[16] = 0  # week
    e[17] = 2024 - 1990  # year
    e[18] = 1  # EDID version
    e[19] = 4  # revision
    # Basic display params: digital input
    e[20] = 0xA5  # digital, 8 bits/color, DisplayPort
    e[21] = H_MM // 10  # max horizontal image size, cm
    e[22] = V_MM // 10  # max vertical image size, cm
    e[23] = 0x78  # gamma 2.2
    e[24] = 0x02  # features: RGB, preferred timing is native
    # Chromaticity (sRGB-ish standard values)
    e[25:35] = bytes([0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54])
    # Established timings: none
    e[35:38] = bytes([0x00, 0x00, 0x00])
    # Standard timings: all unused
    for i in range(38, 54):
        e[i] = 0x01
    # Four 18-byte descriptors
    e[54:72] = detailed_timing()
    e[72:90] = range_descriptor()
    e[90:108] = text_descriptor(0xFC, "MTT1337")  # monitor name
    e[108:126] = text_descriptor(0xFE, "VDD LowDPI")  # unspecified text
    e[126] = 0  # extension count
    # Checksum: sum of all 128 bytes == 0 mod 256
    e[127] = (256 - (sum(e[0:127]) % 256)) % 256
    assert sum(e) % 256 == 0, "checksum failed"
    return bytes(e)


def main() -> int:
    out = sys.argv[1] if len(sys.argv) > 1 else "user_edid.bin"
    data = build()
    with open(out, "wb") as f:
        _ = f.write(data)
    print(f"wrote {out} ({len(data)} bytes)")
    print("hex:", data.hex())
    return 0


if __name__ == "__main__":
    sys.exit(main())
