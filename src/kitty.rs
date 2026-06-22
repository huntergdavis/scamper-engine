//! Kitty graphics protocol frame encoder (PROJECT_PLAN.md §4.4).
//!
//! - Transmit+display (`a=T`) RGB24 (`f=24`), alpha stripped after compositing.
//! - Pinned image id `i=1`, placement `p=1` → same-id re-transmit is self-cleaning
//!   (terminal frees the prior image+placement) so there's no leak and no flicker;
//!   we never send a per-frame delete.
//! - `q=2` suppresses *failure* responses (NOT all — `q=1` suppresses OK); we also
//!   never solicit an OK, and the input parser discards stray `_G` replies.
//! - `C=1` keeps the cursor put.
//! - base64 chunked at 4096 bytes (a multiple of 4); only the final chunk carries `m=0`.
//! - The whole frame is written in one burst → partial transmissions are never shown
//!   (atomic presentation; the protocol has no double-buffer).

pub const IMAGE_ID: u32 = 1;
pub const PLACEMENT_ID: u32 = 1;
const CHUNK: usize = 4096;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Fused RGBA→RGB→base64: read an RGBA8 buffer, drop alpha, emit base64 ASCII of
/// the RGB stream into `b64` (cleared first). RGB length is a multiple of 3, so the
/// output is a clean multiple of 4 with no padding.
pub fn rgba_to_rgb_base64(rgba: &[u8], b64: &mut Vec<u8>) {
    b64.clear();
    b64.reserve(rgba.len() / 4 * 4); // 3 bytes/px -> 4 chars/px
    let mut acc: u32 = 0;
    let mut n = 0u8;
    let mut i = 0;
    while i + 3 < rgba.len() || i + 2 < rgba.len() {
        // read R,G,B (skip A at i+3)
        for k in 0..3 {
            acc = (acc << 8) | rgba[i + k] as u32;
            n += 1;
            if n == 3 {
                b64.push(B64[((acc >> 18) & 63) as usize]);
                b64.push(B64[((acc >> 12) & 63) as usize]);
                b64.push(B64[((acc >> 6) & 63) as usize]);
                b64.push(B64[(acc & 63) as usize]);
                acc = 0;
                n = 0;
            }
        }
        i += 4;
    }
}

/// Append the full Kitty transmit+display command(s) for a base64 RGB payload.
///
/// When `cols`/`rows` are nonzero, the image is *displayed* scaled to fit that
/// many terminal cells (`c=`/`r=`), so we can transmit a small internal image and
/// let the terminal upscale it to fill the window — slashing per-frame bandwidth.
/// Pass `0, 0` to display at native pixel size.
pub fn encode_frame(out: &mut Vec<u8>, width: usize, height: usize, cols: usize, rows: usize, b64: &[u8]) {
    let disp = if cols > 0 && rows > 0 {
        format!(",c={cols},r={rows}")
    } else {
        String::new()
    };
    let mut off = 0;
    let mut first = true;
    // Always emit at least one command (handles empty/edge sizes gracefully).
    loop {
        let remaining = b64.len() - off;
        let n = remaining.min(CHUNK);
        let last = off + n >= b64.len();
        let m = if last { 0 } else { 1 };
        if first {
            out.extend_from_slice(
                format!(
                    "\x1b_Ga=T,f=24,i={IMAGE_ID},p={PLACEMENT_ID},s={width},v={height}{disp},q=2,C=1,m={m};"
                )
                .as_bytes(),
            );
            first = false;
        } else {
            out.extend_from_slice(format!("\x1b_Gm={m};").as_bytes());
        }
        out.extend_from_slice(&b64[off..off + n]);
        out.extend_from_slice(b"\x1b\\");
        off += n;
        if last {
            break;
        }
    }
}

/// Convenience: home the cursor and encode a full RGBA frame into `out`,
/// using `scratch` as the reusable base64 buffer.
pub fn present_rgba(
    out: &mut Vec<u8>,
    width: usize,
    height: usize,
    cols: usize,
    rows: usize,
    rgba: &[u8],
    scratch: &mut Vec<u8>,
) {
    out.clear();
    out.extend_from_slice(b"\x1b[H"); // home cursor; image lands at a fixed anchor
    rgba_to_rgb_base64(rgba, scratch);
    encode_frame(out, width, height, cols, rows, scratch);
}

/// Delete image id=1 and its placements (use on resize/reconfigure).
pub fn delete_image() -> &'static [u8] {
    b"\x1b_Ga=d,d=I,i=1\x1b\\"
}

/// Delete all transmitted images (use on teardown).
pub fn delete_all() -> &'static [u8] {
    b"\x1b_Ga=d,d=A\x1b\\"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_red_pixel() {
        // 1x1 red, RGBA = FF 00 00 FF -> RGB FF 00 00 -> base64 "/wAA"
        let mut b = Vec::new();
        rgba_to_rgb_base64(&[0xFF, 0x00, 0x00, 0xFF], &mut b);
        assert_eq!(b, b"/wAA");
    }

    #[test]
    fn base64_two_pixels() {
        // white + black: FFFFFF -> "////", 000000 -> "AAAA"
        let mut b = Vec::new();
        rgba_to_rgb_base64(&[255, 255, 255, 255, 0, 0, 0, 255], &mut b);
        assert_eq!(b, b"////AAAA");
    }

    #[test]
    fn golden_single_frame() {
        let mut b = Vec::new();
        rgba_to_rgb_base64(&[0xFF, 0x00, 0x00, 0xFF], &mut b);
        let mut out = Vec::new();
        encode_frame(&mut out, 1, 1, 0, 0, &b);
        assert_eq!(out, b"\x1b_Ga=T,f=24,i=1,p=1,s=1,v=1,q=2,C=1,m=0;/wAA\x1b\\");
    }

    #[test]
    fn chunking_multi() {
        // Build a payload > 4096 b64 bytes: need > 3072 RGB bytes = >1024 px.
        let px = 2000;
        let rgba = vec![0u8; px * 4];
        let mut b = Vec::new();
        rgba_to_rgb_base64(&rgba, &mut b);
        assert_eq!(b.len(), px * 4); // 4 chars/px
        let mut out = Vec::new();
        encode_frame(&mut out, px, 1, 0, 0, &b);
        // first command has control keys + m=1, last has m=0
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("a=T,f=24,i=1,p=1"));
        assert!(s.contains("m=1;"));
        assert!(s.contains("m=0;"));
        // number of chunks = ceil(8000/4096) = 2
        assert_eq!(s.matches("\x1b_G").count(), 2);
    }
}
