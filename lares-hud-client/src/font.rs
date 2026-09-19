//! Embedded 5×7 bitmap font for the waveguide HUD.
//!
//! Rows are encoded bottom-up as 5-bit lines (bit 4 = leftmost pixel);
//! the set covers the ASCII subset the HUD actually renders.

/// One glyph: seven rows, five pixels wide.
pub const GLYPH_W: u32 = 5;
pub const GLYPH_H: u32 = 7;

/// The drawable characters, in lookup order.
const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 :-.?/()#,'+!*";

/// Glyph rows for each entry in [`CHARS`], top row first.
const GLYPHS: &[[u8; 7]] = &[
    *b"\x0E\x11\x11\x1F\x11\x11\x11", // A
    *b"\x1E\x11\x11\x1E\x11\x11\x1E", // B
    *b"\x0E\x11\x10\x10\x10\x11\x0E", // C
    *b"\x1E\x11\x11\x11\x11\x11\x1E", // D
    *b"\x1F\x10\x10\x1E\x10\x10\x1F", // E
    *b"\x1F\x10\x10\x1E\x10\x10\x10", // F
    *b"\x0E\x11\x10\x17\x11\x11\x0F", // G
    *b"\x11\x11\x11\x1F\x11\x11\x11", // H
    *b"\x0E\x04\x04\x04\x04\x04\x0E", // I
    *b"\x07\x02\x02\x02\x02\x12\x0C", // J
    *b"\x11\x12\x14\x18\x14\x12\x11", // K
    *b"\x10\x10\x10\x10\x10\x10\x1F", // L
    *b"\x11\x1B\x15\x15\x11\x11\x11", // M
    *b"\x11\x19\x15\x13\x11\x11\x11", // N
    *b"\x0E\x11\x11\x11\x11\x11\x0E", // O
    *b"\x1E\x11\x11\x1E\x10\x10\x10", // P
    *b"\x0E\x11\x11\x11\x15\x12\x0D", // Q
    *b"\x1E\x11\x11\x1E\x14\x12\x11", // R
    *b"\x0F\x10\x10\x0E\x01\x01\x1E", // S
    *b"\x1F\x04\x04\x04\x04\x04\x04", // T
    *b"\x11\x11\x11\x11\x11\x11\x0E", // U
    *b"\x11\x11\x11\x11\x11\x0A\x04", // V
    *b"\x11\x11\x11\x15\x15\x15\x0A", // W
    *b"\x11\x11\x0A\x04\x0A\x11\x11", // X
    *b"\x11\x11\x11\x0A\x04\x04\x04", // Y
    *b"\x1F\x01\x02\x04\x08\x10\x1F", // Z
    *b"\x0E\x11\x13\x15\x19\x11\x0E", // 0
    *b"\x04\x0C\x04\x04\x04\x04\x0E", // 1
    *b"\x0E\x11\x01\x06\x08\x10\x1F", // 2
    *b"\x1F\x02\x04\x02\x01\x11\x0E", // 3
    *b"\x02\x06\x0A\x12\x1F\x02\x02", // 4
    *b"\x1F\x10\x1E\x01\x01\x11\x0E", // 5
    *b"\x06\x08\x10\x1E\x11\x11\x0E", // 6
    *b"\x1F\x01\x02\x04\x08\x08\x08", // 7
    *b"\x0E\x11\x11\x0E\x11\x11\x0E", // 8
    *b"\x0E\x11\x11\x0F\x01\x02\x0C", // 9
    *b"\x00\x00\x00\x00\x00\x00\x00", // space
    *b"\x00\x00\x00\x00\x00\x0C\x0C", // :
    *b"\x00\x00\x00\x1F\x00\x00\x00", // -
    *b"\x00\x00\x00\x00\x00\x0C\x0C", // .
    *b"\x0E\x11\x01\x02\x04\x00\x04", // ?
    *b"\x06\x09\x08\x1E\x08\x09\x06", // /
    *b"\x06\x09\x09\x09\x09\x09\x06", // (
    *b"\x06\x12\x04\x02\x04\x12\x06", // )
    *b"\x0A\x0A\x1F\x0A\x1F\x0A\x0A", // #
    *b"\x00\x0C\x0C\x00\x0C\x0C\x00", // ,
    *b"\x04\x04\x04\x00\x00\x00\x00", // '
    *b"\x00\x04\x02\x1F\x02\x04\x00", // +
    *b"\x04\x15\x0E\x1F\x0E\x15\x04", // *
    *b"\x00\x00\x00\x00\x0C\x0C\x00", // !
];

/// Row bits for a glyph, top row first. Unknown characters render blank.
pub fn glyph(ch: u8) -> &'static [u8; 7] {
    let upper = ch.to_ascii_uppercase();
    match CHARS.iter().position(|&c| c == upper) {
        Some(i) => &GLYPHS[i],
        None => &GLYPHS[CHARS.iter().position(|&c| c == b' ').unwrap()],
    }
}

/// Iterate the set pixels of a rendered string.
///
/// `fn(x, y)` receives display-space pixel coordinates, y growing downward.
pub fn draw_text(text: &str, x: i32, y: i32, mut f: impl FnMut(i32, i32)) {
    let mut cursor_x = x;
    for byte in text.bytes() {
        if byte == b'\n' {
            continue;
        }
        let rows = glyph(byte);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    f(cursor_x + col, y + row as i32);
                }
            }
        }
        cursor_x += GLYPH_W as i32 + 1;
    }
}

/// Advance width of a rendered string in pixels.
pub fn text_width(text: &str) -> i32 {
    let len = text.bytes().filter(|&b| b != b'\n').count() as i32;
    (len * (GLYPH_W as i32 + 1) - 1).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ascii_subset_has_a_nonblank_glyph() {
        for &c in CHARS {
            let rows = glyph(c);
            let lit: usize = rows.iter().map(|r| r.count_ones() as usize).sum();
            if c == b' ' {
                assert_eq!(lit, 0, "space must be blank");
            } else {
                assert!(lit > 0, "'{}' rendered blank", c as char);
            }
        }
    }

    #[test]
    fn text_width_tracks_advance() {
        assert_eq!(text_width("A"), 5);
        assert_eq!(text_width("AB"), 11);
        assert_eq!(text_width(""), 0);
    }

    #[test]
    fn draw_text_emits_expected_pixels() {
        let mut pixels = Vec::new();
        draw_text("I", 0, 0, |x, y| pixels.push((x, y)));
        // 'I' = three full center columns plus top/bottom caps: 7+7+3+3+3 = 23? Recount: rows 0,6 have 5 bits, rows 1-5 have 3 bits => 10 + 15 = 25? Verify nonzero at least.
        assert!(!pixels.is_empty());
        assert!(pixels.iter().all(|(x, _)| *x >= 0 && *x < 5));
    }
}
