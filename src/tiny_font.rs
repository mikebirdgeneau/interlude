use fontdue::Font;
use std::sync::OnceLock;

const FONT_DATA: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
fn font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        Font::from_bytes(FONT_DATA, fontdue::FontSettings::default())
            .expect("Failed to load embedded Inter font")
    })
}

pub fn line_height_size(size: f32) -> i32 {
    let font = font();
    if let Some(metrics) = font.horizontal_line_metrics(size) {
        (metrics.ascent - metrics.descent + metrics.line_gap).round() as i32
    } else {
        (size * 1.3).round() as i32
    }
}

pub fn line_ascent_size(size: f32) -> i32 {
    let font = font();
    font.horizontal_line_metrics(size)
        .map(|metrics| {
            let leading = (metrics.line_gap / 2.0).round() as i32;
            metrics.ascent.round() as i32 + leading
        })
        .unwrap_or(size.round() as i32)
}

pub fn text_width_size(text: &str, size: f32) -> i32 {
    let font = font();
    let mut width = 0i32;
    for ch in text.chars() {
        if ch == '\n' {
            break;
        }
        let (metrics, _) = font.rasterize(ch, size);
        width += metrics.advance_width.round() as i32;
    }
    width
}

pub fn draw_text_rgba_size(
    buf: &mut [u8],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    text: &str,
    rgba: [u8; 4],
    size: f32,
) {
    let font = font();
    let mut pen_x = x;
    let mut pen_y = y;

    for ch in text.chars() {
        if ch == '\n' {
            pen_x = x;
            pen_y += line_height_size(size);
            continue;
        }

        let (metrics, bitmap) = font.rasterize(ch, size);
        let glyph_x = pen_x + metrics.xmin;
        let glyph_y = pen_y - metrics.height as i32 - metrics.ymin;

        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let alpha = bitmap[row * metrics.width + col];
                if alpha == 0 {
                    continue;
                }

                let px = glyph_x + col as i32;
                let py = glyph_y + row as i32;
                if px < 0 || py < 0 || (px as u32) >= width || (py as u32) >= height {
                    continue;
                }

                let idx = ((py as u32 * width + px as u32) * 4) as usize;
                blend_pixel(&mut buf[idx..idx + 4], rgba, alpha);
            }
        }

        pen_x += metrics.advance_width.round() as i32;
    }
}

fn blend_pixel(dst: &mut [u8], rgba: [u8; 4], alpha: u8) {
    let a = (alpha as u16 * rgba[3] as u16) / 255;
    let inv = 255u16.saturating_sub(a);
    dst[0] = ((rgba[0] as u16 * a + dst[0] as u16 * inv) / 255) as u8;
    dst[1] = ((rgba[1] as u16 * a + dst[1] as u16 * inv) / 255) as u8;
    dst[2] = ((rgba[2] as u16 * a + dst[2] as u16 * inv) / 255) as u8;
    dst[3] = 255;
}
