//! `scamp` — the game binary. (Scaffold; the real game loop lands in later steps.)

use scamper::{Framebuffer, Rgba};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("pngtest") => {
            let path = args.get(2).map(|s| s.as_str()).unwrap_or("scamp_test.png");
            png_selftest(path);
        }
        _ => {
            eprintln!("scamp — terminal platformer (scaffold)");
            eprintln!("usage: scamp pngtest [out.png]");
        }
    }
}

/// Render a tiny scene to a PNG so we can verify the framebuffer + PNG path.
fn png_selftest(path: &str) {
    let (w, h) = (320usize, 180usize);
    let mut fb = Framebuffer::new(w, h);
    fb.clear(Rgba::rgb(24, 26, 38)); // night sky
    // ground
    fb.fill_rect(0, 150, w as i32, 30, Rgba::rgb(40, 70, 50));
    // a floating platform
    fb.fill_rect(120, 110, 80, 10, Rgba::rgb(90, 90, 110));
    // the "scamp" (player box)
    fb.fill_rect(60, 120, 14, 18, Rgba::rgb(240, 180, 60));
    fb.stroke_rect(60, 120, 14, 18, Rgba::rgb(255, 230, 140));
    // a velocity vector (debug-overlay style)
    fb.line(67, 129, 95, 115, Rgba::rgb(255, 80, 80));
    scamper::png::write_file(path, w, h, &fb.px).expect("write png");
    eprintln!("wrote {path} ({w}x{h})");
}
