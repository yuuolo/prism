//! Live speed in the tray: the tooltip everywhere, the menu bar text on
//! macOS, and (opt-in) on Windows an icon drawn with the two speeds.
//! Reads the running core's `/traffic` stream.

// The drawn icon is for Windows; macOS shows text instead.
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::time::Duration;

#[cfg(not(target_os = "macos"))]
use tauri::image::Image;
use tauri::{AppHandle, Manager};

use crate::AppState;

/// `1.2M`, `85K`, `999B`: at most 4 characters, per second.
pub fn compact(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if n < 1000 {
        format!("{n}B")
    } else if f < 10.0 * K {
        format!("{:.1}K", f / K)
    } else if f < 1000.0 * K {
        format!("{:.0}K", f / K)
    } else if f < 10.0 * K * K {
        format!("{:.1}M", f / K / K)
    } else if f < 1000.0 * K * K {
        format!("{:.0}M", f / K / K)
    } else {
        format!("{:.1}G", f / K / K / K)
    }
}

/// 3×5 glyphs, one row per entry, bit 2 = left column.
fn glyph(c: char) -> Option<(usize, [u8; 5])> {
    Some(match c {
        '0' => (3, [7, 5, 5, 5, 7]),
        '1' => (3, [2, 6, 2, 2, 7]),
        '2' => (3, [7, 1, 7, 4, 7]),
        '3' => (3, [7, 1, 7, 1, 7]),
        '4' => (3, [5, 5, 7, 1, 1]),
        '5' => (3, [7, 4, 7, 1, 7]),
        '6' => (3, [7, 4, 7, 5, 7]),
        '7' => (3, [7, 1, 1, 1, 1]),
        '8' => (3, [7, 5, 7, 5, 7]),
        '9' => (3, [7, 5, 7, 1, 7]),
        '.' => (1, [0, 0, 0, 0, 1]),
        'B' => (3, [6, 5, 6, 5, 6]),
        'K' => (3, [5, 5, 6, 5, 5]),
        'M' => (3, [5, 7, 7, 5, 5]),
        'G' => (3, [7, 4, 5, 5, 7]),
        _ => return None,
    })
}

const SIZE: usize = 32;
/// Each font pixel is a 2×2 block, so the 16 px tray size shows it 1:1.
const SCALE: usize = 2;

fn draw_line(rgba: &mut [u8], text: &str, top: usize, color: [u8; 3]) {
    let glyphs: Vec<(usize, [u8; 5])> = text.chars().filter_map(glyph).collect();
    let width: usize = glyphs.iter().map(|(w, _)| w * SCALE).sum::<usize>() + glyphs.len().saturating_sub(1) * SCALE;
    let mut x0 = SIZE.saturating_sub(width) / 2;
    for (w, rows) in glyphs {
        for (ry, bits) in rows.iter().enumerate() {
            for cx in 0..w {
                if bits >> (w - 1 - cx) & 1 == 0 {
                    continue;
                }
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        let (x, y) = (x0 + cx * SCALE + dx, top + ry * SCALE + dy);
                        if x < SIZE && y < SIZE {
                            let i = (y * SIZE + x) * 4;
                            rgba[i..i + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
                        }
                    }
                }
            }
        }
        x0 += (w + 1) * SCALE;
    }
}

/// A 32×32 icon: upload on top, download below, on a dark rounded square.
pub fn render(up: u64, down: u64) -> Vec<u8> {
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Rounded corners (radius 5).
            let (cx, cy) = (x.min(SIZE - 1 - x), y.min(SIZE - 1 - y));
            if cx < 5 && cy < 5 && (5 - cx) * (5 - cx) + (5 - cy) * (5 - cy) > 26 {
                continue;
            }
            let i = (y * SIZE + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[27, 29, 39, 240]);
        }
    }
    draw_line(&mut rgba, &compact(up), 4, [178, 166, 255]);
    draw_line(&mut rgba, &compact(down), 18, [64, 208, 255]);
    rgba
}

/// Pixels of the menu bar icon: 18 pt, the height macOS gives it, at 2×.
#[cfg(any(target_os = "macos", test))]
pub const MENU_SIZE: usize = 36;

/// macOS menu bar icon: the logo's prism (a triangle with the light ray
/// cut out), black on transparent, for a template image that macOS tints
/// for light and dark menu bars. Coordinates are the 1024 px app icon's;
/// 4×4 samples per pixel for smooth edges.
#[cfg(any(target_os = "macos", test))]
pub fn menu_bar_icon() -> Vec<u8> {
    type P = (f64, f64);
    const PRISM: [P; 3] = [(512.0, 230.0), (269.0, 770.0), (755.0, 770.0)];
    const RAY: [P; 3] = [(516.0, 500.0), (634.0, 500.0), (677.0, 598.0)];
    let inside = |p: P, t: &[P; 3]| {
        let side = |a: P, b: P| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        let (d1, d2, d3) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
        !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
    };
    // The prism's height (540) fills the icon less a 2 px margin, centred.
    let scale = (MENU_SIZE - 4) as f64 / 540.0;
    let (x0, y0) = (512.0 - MENU_SIZE as f64 / 2.0 / scale, 230.0 - 2.0 / scale);
    let mut rgba = vec![0u8; MENU_SIZE * MENU_SIZE * 4];
    for y in 0..MENU_SIZE {
        for x in 0..MENU_SIZE {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let p = (x0 + (x as f64 + (sx as f64 + 0.5) / 4.0) / scale, y0 + (y as f64 + (sy as f64 + 0.5) / 4.0) / scale);
                    if inside(p, &PRISM) && !inside(p, &RAY) {
                        hits += 1;
                    }
                }
            }
            rgba[(y * MENU_SIZE + x) * 4 + 3] = (hits * 255 / 16) as u8;
        }
    }
    rgba
}

fn tooltip(up: u64, down: u64) -> String {
    format!("Prism\n↑ {}/s  ↓ {}/s", compact(up), compact(down))
}

/// Shows (`Some`) or clears the speed.
fn show(app: &AppHandle, speed: Option<(u64, u64)>, icon_mode: bool, drawn: &mut bool) {
    let Some(tray) = app.tray_by_id("main") else { return };
    let _ = tray.set_tooltip(Some(match speed {
        Some((u, d)) => tooltip(u, d),
        None => "Prism".to_string(),
    }));
    // macOS: text next to the menu bar icon.
    #[cfg(target_os = "macos")]
    {
        let title = speed.filter(|_| icon_mode).map(|(u, d)| format!("↑{} ↓{}", compact(u), compact(d)));
        let _ = tray.set_title(title);
        *drawn = false;
    }
    #[cfg(not(target_os = "macos"))]
    match speed {
        Some((u, d)) if icon_mode => {
            let _ = tray.set_icon(Some(Image::new_owned(render(u, d), SIZE as u32, SIZE as u32)));
            *drawn = true;
        }
        _ if *drawn => {
            let _ = tray.set_icon(app.default_window_icon().cloned());
            *drawn = false;
        }
        _ => {}
    }
}

/// Follows the core's `/traffic` stream for as long as Prism runs.
pub async fn run(app: AppHandle) {
    let mut drawn = false;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let st = app.state::<AppState>();
        if st.core.running().await.is_none() {
            show(&app, None, false, &mut drawn);
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        let s = st.settings();
        let client = match reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(3)).build() {
            Ok(c) => c,
            Err(_) => continue,
        };
        let url = format!("http://127.0.0.1:{}/traffic", s.controller_port);
        let Ok(mut resp) = client.get(&url).bearer_auth(&s.secret).send().await else { continue };
        let mut buf = Vec::new();
        // A chunk at least every few seconds, or the stream is stale.
        while let Ok(Ok(Some(chunk))) = tokio::time::timeout(Duration::from_secs(5), resp.chunk()).await {
            buf.extend_from_slice(&chunk);
            while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=nl).collect();
                let Ok(v) = serde_json::from_slice::<serde_json::Value>(&line) else { continue };
                let (up, down) = (v["up"].as_u64().unwrap_or(0), v["down"].as_u64().unwrap_or(0));
                let icon_mode = app.state::<AppState>().settings().tray_speed;
                show(&app, Some((up, down)), icon_mode, &mut drawn);
            }
        }
        show(&app, None, false, &mut drawn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_fits_four_chars() {
        for (n, s) in [(0, "0B"), (999, "999B"), (1000, "1.0K"), (10 * 1024, "10K"), (999 * 1024, "999K"), (1_300_000, "1.2M"), (50 << 20, "50M"), (3 << 30, "3.0G")] {
            assert_eq!(compact(n), s);
        }
        for n in [0u64, 512, 9_999, 123_456, 999_999, 1 << 20, 999 << 20, 5 << 30] {
            let c = compact(n);
            assert!(c.len() <= 4, "{c}");
            assert!(c.chars().all(|ch| glyph(ch).is_some()), "{c}");
        }
    }

    #[test]
    fn menu_bar_icon_is_the_prism() {
        let px = menu_bar_icon();
        assert_eq!(px.len(), MENU_SIZE * MENU_SIZE * 4);
        let a = |x: usize, y: usize| px[(y * MENU_SIZE + x) * 4 + 3];
        assert!(px.chunks(4).all(|p| p[..3] == [0, 0, 0]), "black only: macOS tints a template");
        assert_eq!((a(0, 0), a(MENU_SIZE - 1, 0), a(0, MENU_SIZE - 1)), (0, 0, 0), "corners are clear");
        assert_eq!(a(MENU_SIZE / 2, MENU_SIZE * 3 / 4), 255, "body is solid");
        assert_eq!(a(MENU_SIZE / 2, 0), 0, "a margin above the apex");
        // The ray is cut out: clear inside the wedge, solid left of it.
        assert_eq!(a(23, 18), 0, "ray");
        assert_eq!(a(14, 18), 255, "left of the ray");
        if let Some(path) = std::env::var_os("PRISM_DUMP_MENU_ICON") {
            std::fs::write(path, &px).unwrap();
        }
    }

    #[test]
    fn icon_draws_both_lines() {
        let px = render(1_300_000, 50 << 20);
        assert_eq!(px.len(), 32 * 32 * 4);
        let lit = |y0: usize, y1: usize, c: u8| (y0..y1).flat_map(|y| (0..32).map(move |x| (y * 32 + x) * 4)).filter(|&i| px[i] == c).count();
        assert!(lit(4, 14, 178) > 20, "upload text");
        assert!(lit(18, 28, 64) > 20, "download text");
        assert_eq!(px[3], 0, "corner is transparent");
        if std::env::var_os("PRISM_DUMP_ICON").is_some() {
            use base64::Engine;
            for (u, d) in [(1_300_000u64, 50u64 << 20), (850 * 1024, 9_999), (0, 512)] {
                println!("ICON {}", base64::engine::general_purpose::STANDARD.encode(render(u, d)));
            }
        }
    }
}
