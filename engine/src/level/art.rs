//! Tile art: a distinct visual style for every [`TileKind`], across every fidelity
//! tier (CAMPAIGN_PLAN.md §6). One source — a 16×16 px pattern drawn into the
//! framebuffer — feeds all four backends: the kitty tier shows the pixels, the
//! text tier its half-block downsample, and the ascii/mono tiers a brightness ramp.
//!
//! The hard constraint is **mono**: with no color, tiles must still read apart.
//! Since a 16px tile maps to a 4×2 grid of character cells (the engine's
//! dimensional parity), each kind is designed so that *cell-scale luma layout* is
//! its signature — grass-on-soil vs. brick lattice vs. asymmetric pipe shading vs.
//! a top-only platform vs. a wavy hazard. Color (themes) then separates them
//! further on the colored tiers. Nothing here is sampled from any external art.

use super::ir::TileKind;
use crate::framebuffer::{Framebuffer, Rgba};

/// Tile edge length in framebuffer pixels (matches `world::TILE`).
pub const TILE: i32 = 16;

/// Level visual themes — they re-tint the same tile *patterns*, mirroring how the
/// source levels swap one atlas layout across themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Overworld,
    Underground,
    Underwater,
    Castle,
    Snow,
}

impl Theme {
    pub fn from_str(s: &str) -> Theme {
        match s.to_ascii_lowercase().as_str() {
            "underground" | "cave" => Theme::Underground,
            "underwater" | "water" => Theme::Underwater,
            "castle" | "bath" | "bathhouse" => Theme::Castle,
            "snow" | "ice" => Theme::Snow,
            _ => Theme::Overworld,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Theme::Overworld => "overworld",
            Theme::Underground => "underground",
            Theme::Underwater => "underwater",
            Theme::Castle => "castle",
            Theme::Snow => "snow",
        }
    }
    /// Cycle order for the preview tool.
    pub const ALL: [Theme; 5] = [Theme::Overworld, Theme::Underground, Theme::Underwater, Theme::Castle, Theme::Snow];
}

/// Named colors a theme provides to the tile patterns.
#[derive(Clone, Copy)]
pub struct Palette {
    pub sky: Rgba,         // background behind non-solid tiles
    pub ground: Rgba,      // soil body
    pub ground_top: Rgba,  // surface cap (grass/snow/stone lip)
    pub ground_dark: Rgba, // soil speckle
    pub brick: Rgba,
    pub mortar: Rgba,
    pub block: Rgba,       // question/coin-block body
    pub block_rivet: Rgba,
    pub accent: Rgba,      // the "?" / coin
    pub pipe: Rgba,
    pub platform: Rgba,
    pub hazard_a: Rgba,    // hazard base (deep)
    pub hazard_b: Rgba,    // hazard crest (hot/foam)
    pub deco: Rgba,        // faint scenery
    pub hint: Rgba,        // hidden-block ghost outline
}

const fn c(r: u8, g: u8, b: u8) -> Rgba {
    Rgba::rgb(r, g, b)
}

pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Overworld => Palette {
            sky: c(24, 28, 44),
            ground: c(176, 112, 64),
            ground_top: c(96, 196, 88),
            ground_dark: c(126, 74, 40),
            brick: c(196, 98, 56),
            mortar: c(86, 38, 22),
            block: c(232, 184, 64),
            block_rivet: c(120, 82, 20),
            accent: c(70, 44, 12),
            pipe: c(64, 176, 76),
            platform: c(206, 154, 92),
            hazard_a: c(176, 48, 20),
            hazard_b: c(255, 168, 48),
            deco: c(70, 158, 74),
            hint: c(70, 78, 104),
        },
        Theme::Underground => Palette {
            sky: c(8, 10, 18),
            ground: c(58, 92, 150),
            ground_top: c(96, 146, 214),
            ground_dark: c(36, 60, 104),
            brick: c(70, 110, 162),
            mortar: c(26, 44, 82),
            block: c(232, 184, 64),
            block_rivet: c(120, 82, 20),
            accent: c(40, 36, 12),
            pipe: c(64, 176, 76),
            platform: c(96, 124, 176),
            hazard_a: c(150, 40, 24),
            hazard_b: c(252, 150, 40),
            deco: c(70, 104, 168),
            hint: c(40, 52, 84),
        },
        Theme::Underwater => Palette {
            sky: c(10, 36, 58),
            ground: c(40, 132, 132),
            ground_top: c(96, 206, 196),
            ground_dark: c(28, 96, 100),
            brick: c(48, 140, 140),
            mortar: c(20, 72, 78),
            block: c(228, 196, 96),
            block_rivet: c(110, 92, 36),
            accent: c(30, 60, 60),
            pipe: c(72, 184, 150),
            platform: c(88, 168, 168),
            hazard_a: c(132, 36, 96),
            hazard_b: c(228, 120, 196),
            deco: c(72, 166, 158),
            hint: c(36, 86, 96),
        },
        Theme::Castle => Palette {
            sky: c(16, 14, 18),
            ground: c(120, 118, 132),
            ground_top: c(168, 166, 180),
            ground_dark: c(78, 76, 90),
            brick: c(116, 112, 128),
            mortar: c(54, 52, 64),
            block: c(228, 184, 72),
            block_rivet: c(112, 80, 24),
            accent: c(54, 46, 20),
            pipe: c(96, 132, 110),
            platform: c(140, 138, 152),
            hazard_a: c(186, 52, 18),
            hazard_b: c(255, 176, 56),
            deco: c(110, 108, 122),
            hint: c(60, 58, 70),
        },
        Theme::Snow => Palette {
            sky: c(40, 52, 78),
            ground: c(178, 196, 220),
            ground_top: c(238, 246, 255),
            ground_dark: c(140, 158, 188),
            brick: c(166, 186, 212),
            mortar: c(96, 116, 148),
            block: c(232, 192, 84),
            block_rivet: c(120, 90, 28),
            accent: c(72, 60, 24),
            pipe: c(86, 178, 132),
            platform: c(200, 214, 234),
            hazard_a: c(120, 150, 210),
            hazard_b: c(214, 232, 255),
            deco: c(150, 172, 206),
            hint: c(96, 112, 142),
        },
    }
}

/// Scale a color toward black by `num/den` (a shade).
fn shade(col: Rgba, num: u32, den: u32) -> Rgba {
    let f = |v: u8| (v as u32 * num / den).min(255) as u8;
    Rgba::rgb(f(col.r), f(col.g), f(col.b))
}
/// Mix a color toward white by `num/den` (a tint/highlight).
fn light(col: Rgba, num: u32, den: u32) -> Rgba {
    let f = |v: u8| (v as u32 + (255 - v as u32) * num / den).min(255) as u8;
    Rgba::rgb(f(col.r), f(col.g), f(col.b))
}

/// All kinds in a stable order (for the preview tool + tests).
pub const KINDS: [TileKind; 9] = [
    TileKind::Ground,
    TileKind::Platform,
    TileKind::Brick,
    TileKind::CoinBrick,
    TileKind::Question,
    TileKind::Hidden,
    TileKind::Pipe,
    TileKind::Hazard,
    TileKind::Deco,
];

/// Draw one tile's 16×16 art with its top-left at (`ox`,`oy`). Non-solid kinds
/// (platform underside, deco, hidden) leave their empty pixels untouched so the
/// background/sky shows through.
pub fn draw_tile(fb: &mut Framebuffer, ox: i32, oy: i32, kind: TileKind, p: &Palette) {
    match kind {
        TileKind::Ground => {
            fb.fill_rect(ox, oy, TILE, TILE, p.ground);
            fb.fill_rect(ox, oy, TILE, 4, p.ground_top); // bright surface cap
            fb.fill_rect(ox, oy + 4, TILE, 1, shade(p.ground, 3, 4));
            // soil speckle (low) for a non-flat body
            fb.fill_rect(ox + 2, oy + 9, 2, 2, p.ground_dark);
            fb.fill_rect(ox + 9, oy + 12, 2, 2, p.ground_dark);
            fb.fill_rect(ox + 12, oy + 8, 2, 2, p.ground_dark);
        }
        TileKind::Brick => draw_brick(fb, ox, oy, p, false),
        TileKind::CoinBrick => draw_brick(fb, ox, oy, p, true),
        TileKind::Question => {
            fb.fill_rect(ox, oy, TILE, TILE, p.block);
            fb.fill_rect(ox, oy, TILE, 2, light(p.block, 1, 2)); // top sheen
            fb.fill_rect(ox, oy + TILE - 2, TILE, 2, shade(p.block, 2, 3));
            for (rx, ry) in [(1, 1), (TILE - 3, 1), (1, TILE - 3), (TILE - 3, TILE - 3)] {
                fb.fill_rect(ox + rx, oy + ry, 2, 2, p.block_rivet); // corner rivets
            }
            // a blocky "?" in the accent color
            fb.fill_rect(ox + 5, oy + 4, 6, 2, p.accent);
            fb.fill_rect(ox + 9, oy + 4, 2, 4, p.accent);
            fb.fill_rect(ox + 7, oy + 8, 2, 2, p.accent);
            fb.fill_rect(ox + 7, oy + 12, 2, 2, p.accent);
        }
        TileKind::Hidden => {
            // a ghost outline only — nearly invisible (materializes when bonked)
            let h = p.hint;
            for (cx, cy) in [(0, 0), (TILE - 3, 0), (0, TILE - 3), (TILE - 3, TILE - 3)] {
                fb.fill_rect(ox + cx, oy + cy, 3, 1, h);
                fb.fill_rect(ox + if cx == 0 { 0 } else { TILE - 1 }, oy + cy, 1, 3, h);
            }
            fb.fill_rect(ox + 7, oy + 7, 2, 2, h); // faint center pip
        }
        TileKind::Pipe => {
            fb.fill_rect(ox, oy, TILE, TILE, p.pipe);
            fb.fill_rect(ox, oy, 4, TILE, light(p.pipe, 2, 5)); // bright left column
            fb.fill_rect(ox + TILE - 4, oy, 4, TILE, shade(p.pipe, 1, 2)); // dark right column
            fb.fill_rect(ox, oy, TILE, 3, light(p.pipe, 1, 3)); // rim
            fb.fill_rect(ox, oy + 3, TILE, 1, shade(p.pipe, 2, 3)); // rim shadow
        }
        TileKind::Platform => {
            // a thin ledge: solid top, empty below (one-way / semisolid)
            fb.fill_rect(ox, oy, TILE, 5, p.platform);
            fb.fill_rect(ox, oy, TILE, 1, light(p.platform, 1, 2));
            fb.fill_rect(ox, oy + 4, TILE, 1, shade(p.platform, 1, 2));
        }
        TileKind::Hazard => {
            // A liquid: a dark pool with bright crest *peaks*. The peaks sit over
            // mono sample columns 0 and 2 and the troughs over 1 and 3, so the top
            // row alternates bright/dark — a signature no solid tile (with its
            // uniform bright cap) shares. Low ripples brighten the trough columns
            // on the bottom row too, so both rows alternate (clearly not ground).
            fb.fill_rect(ox, oy, TILE, TILE, p.hazard_a); // deep pool
            fb.fill_rect(ox, oy, 4, 5, p.hazard_b); // crest peak over sample (0,0)
            fb.fill_rect(ox + 8, oy, 4, 5, p.hazard_b); // crest peak over sample (8,0)
            fb.fill_rect(ox + 4, oy + 8, 4, 4, light(p.hazard_a, 2, 3)); // ripple at sample (4,8)
            fb.fill_rect(ox + 12, oy + 8, 4, 4, light(p.hazard_a, 2, 3)); // ripple at sample (12,8)
        }
        TileKind::Deco => {
            // Faint background tufts — placed so two sit under mono sample columns
            // 1 and 3 on the bottom row, giving deco a "  .   ." signature that's
            // distinct from the (near-empty) hidden block, without being solid.
            let d = p.deco;
            fb.fill_rect(ox + 4, oy + 6, 2, 10, d); // tuft under sample (4,8)
            fb.fill_rect(ox + 12, oy + 6, 2, 10, light(d, 1, 4)); // tuft under sample (12,8)
            fb.fill_rect(ox + 8, oy + 11, 1, 5, d); // a faint sprig (off the sample grid)
        }
    }
}

fn draw_brick(fb: &mut Framebuffer, ox: i32, oy: i32, p: &Palette, coin: bool) {
    fb.fill_rect(ox, oy, TILE, TILE, p.brick);
    fb.fill_rect(ox, oy, TILE, 1, light(p.brick, 1, 3)); // top highlight
    let m = p.mortar;
    fb.fill_rect(ox, oy + 7, TILE, 2, m); // mid horizontal seam
    // Two top-course splits (sample cols 1 & 2 → mortar), so brick reads as a
    // lattice ":  :" — distinct from a platform's solid top row even in mono.
    fb.fill_rect(ox + 3, oy, 2, 7, m);
    fb.fill_rect(ox + 7, oy, 2, 7, m);
    fb.fill_rect(ox + 3, oy + 9, 2, 7, m); // bottom course (offset)
    fb.fill_rect(ox + 11, oy + 9, 2, 7, m);
    if coin {
        // a bright gold coin — wide enough to cover mono sample columns 1 AND 2 on
        // the bottom row, so a coin-brick differs from a plain brick in two cells.
        fb.fill_rect(ox + 4, oy + 5, 8, 6, p.block);
        fb.fill_rect(ox + 5, oy + 6, 6, 4, light(p.block, 1, 2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 4×2 cell-luma signature of a tile — what the mono/ascii ramp samples.
    fn signature(kind: TileKind) -> [u8; 8] {
        signature_in(kind, Theme::Overworld)
    }
    fn signature_in(kind: TileKind, theme: Theme) -> [u8; 8] {
        let p = palette(theme);
        let mut fb = Framebuffer::new(TILE as usize, TILE as usize);
        fb.clear(p.sky);
        draw_tile(&mut fb, 0, 0, kind, &p);
        let luma = |r: u8, g: u8, b: u8| (r as u32 * 54 + g as u32 * 183 + b as u32 * 19) >> 8;
        let mut sig = [0u8; 8];
        for cy in 0..2 {
            for cx in 0..4 {
                // each cell is 4px wide × 8px tall
                let mut acc = 0u32;
                for y in 0..8 {
                    for x in 0..4 {
                        let i = (((cy * 8 + y) * TILE as usize) + (cx * 4 + x)) * 4;
                        acc += luma(fb.px[i], fb.px[i + 1], fb.px[i + 2]) as u32;
                    }
                }
                sig[cy * 4 + cx] = (acc / 32) as u8;
            }
        }
        sig
    }

    // ---- the REAL mono signature: exactly what `MonoBackend` shows ----
    // The mono backend samples ONE pixel per character cell at the cell's top-left
    // (backend::sample with a 4-wide × 2-tall grid over a 16px tile → pixels at
    // x∈{0,4,8,12}, y∈{0,8}), takes Rec.601 luma, and maps through this 5-level
    // ramp. The old test block-AVERAGED with a different luma — a proxy that hid
    // real collisions (e.g. lava vs ground). This mirrors the backend exactly.
    const RAMP_COARSE: &[u8] = b" .:+#";
    fn mono_luma(r: u8, g: u8, b: u8) -> u32 {
        (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000
    }
    fn mono_glyph(l: u32) -> u8 {
        RAMP_COARSE[(l as usize * (RAMP_COARSE.len() - 1)) / 255]
    }
    fn mono_sig(kind: TileKind, theme: Theme) -> [u8; 8] {
        let p = palette(theme);
        let mut fb = Framebuffer::new(TILE as usize, TILE as usize);
        fb.clear(p.sky);
        draw_tile(&mut fb, 0, 0, kind, &p);
        let mut sig = [0u8; 8];
        for cy in 0..2 {
            for cx in 0..4 {
                let (x, y) = (cx * 4, cy * 8); // the backend's sample point per cell
                let i = (y * TILE as usize + x) * 4;
                sig[cy * 4 + cx] = mono_glyph(mono_luma(fb.px[i], fb.px[i + 1], fb.px[i + 2]));
            }
        }
        sig
    }
    fn hamming(a: &[u8; 8], b: &[u8; 8]) -> usize {
        (0..8).filter(|&i| a[i] != b[i]).count()
    }

    #[test]
    #[ignore] // diagnostic: `cargo test dump_mono -- --ignored --nocapture`
    fn dump_mono() {
        for theme in Theme::ALL {
            println!("--- {} ---", theme.name());
            for k in KINDS {
                println!("{:>10?}  {:?}", k, String::from_utf8_lossy(&mono_sig(k, theme)));
            }
        }
    }

    #[test]
    fn mono_tiles_are_visibly_distinct() {
        // The headline requirement: distinct in real black & white, in EVERY theme.
        // We require a margin — at least 2 of the 8 cells differ — so no two kinds
        // are merely borderline-different.
        for theme in Theme::ALL {
            let sigs: Vec<[u8; 8]> = KINDS.iter().map(|&k| mono_sig(k, theme)).collect();
            for i in 0..sigs.len() {
                for j in (i + 1)..sigs.len() {
                    let d = hamming(&sigs[i], &sigs[j]);
                    assert!(
                        d >= 2,
                        "in the {} theme, {:?} and {:?} differ in only {}/8 mono cells:\n  {:?}: {:?}\n  {:?}: {:?}",
                        theme.name(), KINDS[i], KINDS[j], d,
                        KINDS[i], String::from_utf8_lossy(&sigs[i]),
                        KINDS[j], String::from_utf8_lossy(&sigs[j]),
                    );
                }
            }
        }
    }

    #[test]
    fn platform_is_top_heavy_and_ground_is_full() {
        // Platform: bright top row, empty (sky) bottom row.
        let s = signature(TileKind::Platform);
        let top: u32 = s[0..4].iter().map(|&v| v as u32).sum();
        let bot: u32 = s[4..8].iter().map(|&v| v as u32).sum();
        assert!(top > bot + 8, "platform should be top-heavy: {s:?}");
        // Ground: both rows substantial.
        let g = signature(TileKind::Ground);
        assert!(g[4..8].iter().all(|&v| v > 4), "ground body should be filled: {g:?}");
    }

    #[test]
    fn all_themes_render_without_panicking() {
        for t in Theme::ALL {
            let p = palette(t);
            let mut fb = Framebuffer::new(TILE as usize, TILE as usize);
            for k in KINDS {
                fb.clear(p.sky);
                draw_tile(&mut fb, 0, 0, k, &p);
            }
        }
    }
}
