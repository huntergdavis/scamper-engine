//! Minimal, dependency-free PNG encoder (RGBA8, uncompressed/"stored" DEFLATE).
//! Used only for the headless frame-dump path (PROJECT_PLAN.md §4 dev/test) — it
//! lets us write exactly the RGBA the Kitty path would send, as an inspectable file.
//! Not on the live hot path, so simplicity > size.

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in bytes {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

fn push_be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    push_be32(out, data.len() as u32);
    let mut typed = Vec::with_capacity(4 + data.len());
    typed.extend_from_slice(kind);
    typed.extend_from_slice(data);
    out.extend_from_slice(&typed);
    push_be32(out, crc32(&typed));
}

/// Encode an RGBA8 buffer (`px.len() == width*height*4`) as PNG bytes.
pub fn encode_rgba(width: usize, height: usize, px: &[u8]) -> Vec<u8> {
    assert_eq!(px.len(), width * height * 4);

    // Filtered scanlines: each row prefixed with filter byte 0 (None).
    let mut raw = Vec::with_capacity(height * (1 + width * 4));
    let stride = width * 4;
    for y in 0..height {
        raw.push(0u8);
        raw.extend_from_slice(&px[y * stride..(y + 1) * stride]);
    }

    // zlib stream: header + stored DEFLATE blocks + adler32.
    let mut zlib = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 16);
    zlib.push(0x78); // CMF: deflate, 32K window
    zlib.push(0x01); // FLG (makes 0x7801 % 31 == 0)
    let mut off = 0;
    while off < raw.len() {
        let n = (raw.len() - off).min(65535);
        let final_block = off + n >= raw.len();
        zlib.push(if final_block { 1 } else { 0 }); // BFINAL + BTYPE=00 (stored)
        zlib.extend_from_slice(&(n as u16).to_le_bytes()); // LEN
        zlib.extend_from_slice(&(!(n as u16)).to_le_bytes()); // NLEN
        zlib.extend_from_slice(&raw[off..off + n]);
        off += n;
    }
    push_be32(&mut zlib, adler32(&raw));

    let mut out = Vec::new();
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]); // signature

    let mut ihdr = Vec::with_capacity(13);
    push_be32(&mut ihdr, width as u32);
    push_be32(&mut ihdr, height as u32);
    ihdr.push(8); // bit depth
    ihdr.push(6); // color type RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Encode and write to a file.
pub fn write_file(path: &str, width: usize, height: usize, px: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, encode_rgba(width, height, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_vector() {
        // CRC-32 of "IEND" type+empty is a known PNG constant: 0xAE426082.
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn encodes_signature_and_chunks() {
        let px = vec![255u8; 4 * 4 * 4]; // 4x4 white
        let png = encode_rgba(4, 4, &px);
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        // contains IHDR, IDAT, IEND type tags
        let s = &png;
        let find = |tag: &[u8]| s.windows(4).any(|w| w == tag);
        assert!(find(b"IHDR") && find(b"IDAT") && find(b"IEND"));
    }
}
