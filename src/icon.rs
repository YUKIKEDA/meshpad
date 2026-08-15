//! アプリアイコン（`assets/icons/meshpad_icon.ico`）。
//!
//! ICO 内は PNG。ウィンドウ用は最大サイズ、タイトルバーは 16px を使う。

use eframe::egui::{ColorImage, IconData};

const ICO: &[u8] = include_bytes!("../assets/icons/meshpad_icon.ico");

/// ウィンドウ／タスクバー用。ICO 内のいちばん大きい PNG。
pub fn viewport_icon() -> IconData {
    let (w, h, rgba) = decode_png(png_bytes(None));
    IconData {
        rgba,
        width: w,
        height: h,
    }
}

/// タイトルバーの 16px マーク。
pub fn title_color_image() -> ColorImage {
    let (w, h, rgba) = decode_png(png_bytes(Some(16)));
    ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)
}

fn png_bytes(prefer: Option<u32>) -> &'static [u8] {
    png_in_ico(ICO, prefer).unwrap_or_else(|| panic!("meshpad_icon.ico: missing PNG"))
}

fn png_in_ico(ico: &[u8], prefer: Option<u32>) -> Option<&[u8]> {
    if ico.len() < 6 {
        return None;
    }
    if u16::from_le_bytes(ico[0..2].try_into().ok()?) != 0 {
        return None;
    }
    if u16::from_le_bytes(ico[2..4].try_into().ok()?) != 1 {
        return None;
    }
    let n = u16::from_le_bytes(ico[4..6].try_into().ok()?) as usize;
    let mut exact = None;
    let mut largest: Option<(u32, &[u8])> = None;
    for i in 0..n {
        let o = 6 + i * 16;
        if ico.len() < o + 16 {
            return None;
        }
        let w = if ico[o] == 0 { 256 } else { ico[o] as u32 };
        let bytes = u32::from_le_bytes(ico[o + 8..o + 12].try_into().ok()?) as usize;
        let off = u32::from_le_bytes(ico[o + 12..o + 16].try_into().ok()?) as usize;
        let end = off.checked_add(bytes)?;
        let slice = ico.get(off..end)?;
        if !slice.starts_with(&[0x89, b'P', b'N', b'G']) {
            continue;
        }
        if prefer == Some(w) {
            exact = Some(slice);
        }
        if largest.map(|(bw, _)| w > bw).unwrap_or(true) {
            largest = Some((w, slice));
        }
    }
    exact.or(largest.map(|(_, s)| s))
}

fn decode_png(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let img = image::load_from_memory(png)
        .expect("meshpad_icon.ico PNG")
        .into_rgba8();
    (img.width(), img.height(), img.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ico_has_title_and_viewport_pngs() {
        let title = png_in_ico(ICO, Some(16)).expect("16px");
        let large = png_in_ico(ICO, None).expect("largest");
        let (tw, th, _) = decode_png(title);
        let (lw, lh, _) = decode_png(large);
        assert_eq!((tw, th), (16, 16));
        assert!(lw >= 32 && lh >= 32);
    }
}
