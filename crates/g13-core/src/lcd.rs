pub const LCD_WIDTH: usize = 160;
pub const LCD_HEIGHT: usize = 43;
pub const LCD_REPORT_ID: u8 = 0x03;
pub const LCD_REPORT_SIZE: usize = 992;
pub const LCD_HEADER_SIZE: usize = 32;
pub const LCD_BUFFER_SIZE: usize = 960; // 6 pages * 160 cols

/// Framebuffer for the Logitech G13 160x43 monochrome LCD.
///
/// Organization:
/// Total 992 bytes:
/// - Byte 0: Report ID (0x03)
/// - Bytes 1..31: Reserved (0x00)
/// - Bytes 32..991: 960 bytes of graphics memory arranged in 6 pages of 160 columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct G13LcdBuffer {
    data: [u8; LCD_REPORT_SIZE],
}

impl Default for G13LcdBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl G13LcdBuffer {
    /// Create a new, blank framebuffer initialized with Report ID 0x03.
    pub const fn new() -> Self {
        let mut data = [0u8; LCD_REPORT_SIZE];
        data[0] = LCD_REPORT_ID;
        Self { data }
    }

    /// Clear all pixels (turn off).
    pub fn clear(&mut self) {
        self.data[LCD_HEADER_SIZE..].fill(0);
    }

    /// Fill all pixels (turn on).
    pub fn fill(&mut self) {
        self.data[LCD_HEADER_SIZE..].fill(0xFF);
    }

    /// Set an individual pixel state at coordinate (x, y).
    ///
    /// (0, 0) is top-left, (159, 42) is bottom-right.
    pub fn set_pixel(&mut self, x: usize, y: usize, on: bool) {
        if x >= LCD_WIDTH || y >= LCD_HEIGHT {
            return;
        }
        let page = y / 8;
        let bit = y % 8;
        let offset = LCD_HEADER_SIZE + (page * LCD_WIDTH) + x;
        if on {
            self.data[offset] |= 1 << bit;
        } else {
            self.data[offset] &= !(1 << bit);
        }
    }

    /// Read pixel state at coordinate (x, y).
    pub fn get_pixel(&self, x: usize, y: usize) -> bool {
        if x >= LCD_WIDTH || y >= LCD_HEIGHT {
            return false;
        }
        let page = y / 8;
        let bit = y % 8;
        let offset = LCD_HEADER_SIZE + (page * LCD_WIDTH) + x;
        (self.data[offset] & (1 << bit)) != 0
    }

    /// Draw a hollow rectangle.
    pub fn draw_rect(&mut self, x: usize, y: usize, w: usize, h: usize) {
        if w == 0 || h == 0 {
            return;
        }
        let x_end = (x + w - 1).min(LCD_WIDTH - 1);
        let y_end = (y + h - 1).min(LCD_HEIGHT - 1);
        for cur_x in x..=x_end {
            self.set_pixel(cur_x, y, true);
            self.set_pixel(cur_x, y_end, true);
        }
        for cur_y in y..=y_end {
            self.set_pixel(x, cur_y, true);
            self.set_pixel(x_end, cur_y, true);
        }
    }

    /// Draw a horizontal line.
    pub fn draw_hline(&mut self, x: usize, y: usize, length: usize) {
        for cur_x in x..(x + length).min(LCD_WIDTH) {
            self.set_pixel(cur_x, y, true);
        }
    }

    /// Draw a single character (5x7 glyph) at coordinate (x, y).
    ///
    /// ASCII is drawn as is; common French letters (é è ê ë à â ä ç î ï ô ö
    /// ù û ü œ, and their capitals without the accent) are drawn as their
    /// base letter plus the accent in the two free rows above lower-case
    /// letters. Other characters are skipped.
    pub fn draw_char(&mut self, x: usize, y: usize, c: char) {
        self.draw_char_scaled(x, y, c, 1);
    }

    /// Like [`Self::draw_char`], each pixel drawn as a `scale`×`scale` block.
    pub fn draw_char_scaled(&mut self, x: usize, y: usize, c: char, scale: usize) {
        let (base, accent) = decompose(c);
        let ascii = base as usize;
        if !(32..=126).contains(&ascii) {
            return;
        }
        let mut dot = |px: usize, py: usize| {
            for dx in 0..scale {
                for dy in 0..scale {
                    self.set_pixel(x + px * scale + dx, y + py * scale + dy, true);
                }
            }
        };
        let glyph = FONT_5X7[ascii - 32];
        for (col_idx, &col_byte) in glyph.iter().enumerate() {
            for row_idx in 0..7 {
                if (col_byte & (1 << row_idx)) != 0 {
                    dot(col_idx, row_idx);
                }
            }
        }
        for &(px, py) in accent {
            dot(px, py);
        }
    }

    /// Draw a text string at coordinate (x, y).
    /// Each character is 5 pixels wide + 1 pixel spacing.
    pub fn draw_text(&mut self, x: usize, y: usize, text: &str) {
        self.draw_text_scaled(x, y, text, 1);
    }

    /// Draw text with every pixel enlarged `scale` times (character cell
    /// `6·scale` wide, `7·scale` high). Clipped at the right edge.
    pub fn draw_text_scaled(&mut self, x: usize, y: usize, text: &str, scale: usize) {
        let scale = scale.max(1);
        let mut cur_x = x;
        for c in text.chars().filter(|c| drawable(*c)) {
            if cur_x + 5 * scale > LCD_WIDTH {
                break;
            }
            self.draw_char_scaled(cur_x, y, c, scale);
            cur_x += 6 * scale;
        }
    }

    /// Draw text starting `offset` pixels to the left of `x` and clipped to
    /// the columns `x..x + width` (for scrolling titles).
    pub fn draw_text_window(&mut self, x: usize, y: usize, width: usize, offset: usize, text: &str) {
        let mut tmp = Self::new();
        // render on a wide virtual line, then copy the visible window
        let mut col = 0usize;
        for c in text.chars().filter(|c| drawable(*c)) {
            if col + 6 > offset + width + 6 {
                break;
            }
            if col + 6 > offset {
                // draw char at virtual column `col` into tmp, shifted by -offset
                let shifted = col as isize - offset as isize;
                if shifted >= 0 {
                    tmp.draw_char(shifted as usize, 0, c);
                } else {
                    // partially visible on the left: draw then shift
                    let mut one = Self::new();
                    one.draw_char(0, 0, c);
                    for px in 0..6usize {
                        let tx = px as isize + shifted;
                        if tx >= 0 {
                            for py in 0..8 {
                                if one.get_pixel(px, py) {
                                    tmp.set_pixel(tx as usize, py, true);
                                }
                            }
                        }
                    }
                }
            }
            col += 6;
        }
        for px in 0..width.min(LCD_WIDTH.saturating_sub(x)) {
            for py in 0..8 {
                if tmp.get_pixel(px, py) {
                    self.set_pixel(x + px, y + py, true);
                }
            }
        }
    }

    /// Width in pixels of `text` drawn at `scale` (without the trailing gap).
    pub fn text_width(text: &str, scale: usize) -> usize {
        let n = text.chars().filter(|c| drawable(*c)).count();
        if n == 0 { 0 } else { (n * 6 - 1) * scale.max(1) }
    }

    /// Fill (or clear) a rectangle.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, on: bool) {
        for cx in x..(x + w).min(LCD_WIDTH) {
            for cy in y..(y + h).min(LCD_HEIGHT) {
                self.set_pixel(cx, cy, on);
            }
        }
    }

    /// Invert every pixel of a rectangle (white-on-black header bars).
    pub fn invert_rect(&mut self, x: usize, y: usize, w: usize, h: usize) {
        for cx in x..(x + w).min(LCD_WIDTH) {
            for cy in y..(y + h).min(LCD_HEIGHT) {
                let on = self.get_pixel(cx, cy);
                self.set_pixel(cx, cy, !on);
            }
        }
    }

    /// Hollow bar `w`×`h` filled from the left by `ratio` (0.0–1.0).
    pub fn draw_bar(&mut self, x: usize, y: usize, w: usize, h: usize, ratio: f32) {
        self.draw_rect(x, y, w, h);
        if w > 4 && h > 4 {
            let inner = w - 4;
            let filled = ((inner as f32) * ratio.clamp(0.0, 1.0)).round() as usize;
            self.fill_rect(x + 2, y + 2, filled, h - 4, true);
        }
    }

    /// The 960 pixel bytes (6 pages × 160 columns), without the report header.
    pub fn pixels(&self) -> &[u8] {
        &self.data[LCD_HEADER_SIZE..]
    }

    /// Return the full 992-byte report slice ready for HID transmission.
    pub const fn as_report(&self) -> &[u8; LCD_REPORT_SIZE] {
        &self.data
    }
}

// Accent marks drawn above a lower-case glyph (rows 0–1 of the 5x7 cell are
// empty for x-height letters); the cedilla goes just below the baseline.
const ACUTE: &[(usize, usize)] = &[(3, 0), (2, 1)];
const GRAVE: &[(usize, usize)] = &[(1, 0), (2, 1)];
const CIRC: &[(usize, usize)] = &[(2, 0), (1, 1), (3, 1)];
const DIAER: &[(usize, usize)] = &[(1, 0), (3, 0)];
const CEDIL: &[(usize, usize)] = &[(2, 7)];
const TILDE: &[(usize, usize)] = &[(1, 1), (2, 0), (3, 1), (4, 0)];
const NONE: &[(usize, usize)] = &[];

/// Characters the 5x7 font can show (ASCII + the accented letters below).
/// Text functions skip the others (emoji, joiners…) without leaving a gap.
pub fn drawable(c: char) -> bool {
    (' '..='~').contains(&decompose(c).0)
}

/// Base ASCII letter + accent pixels for the French letters we can draw.
fn decompose(c: char) -> (char, &'static [(usize, usize)]) {
    match c {
        'é' => ('e', ACUTE),
        'è' => ('e', GRAVE),
        'ê' => ('e', CIRC),
        'ë' => ('e', DIAER),
        'à' => ('a', GRAVE),
        'â' => ('a', CIRC),
        'ä' => ('a', DIAER),
        'ç' => ('c', CEDIL),
        'î' => ('i', CIRC),
        'ï' => ('i', DIAER),
        'ô' => ('o', CIRC),
        'ö' => ('o', DIAER),
        'ù' => ('u', GRAVE),
        'û' => ('u', CIRC),
        'ü' => ('u', DIAER),
        // Italian and Spanish (LCD texts in those languages)
        'á' => ('a', ACUTE),
        'í' => ('i', ACUTE),
        'ì' => ('i', GRAVE),
        'ó' => ('o', ACUTE),
        'ò' => ('o', GRAVE),
        'ú' => ('u', ACUTE),
        'ñ' => ('n', TILDE),
        'É' | 'È' | 'Ê' | 'Ë' => ('E', NONE),
        'À' | 'Â' | 'Ä' | 'Á' => ('A', NONE),
        'Ç' => ('C', NONE),
        'Î' | 'Ï' | 'Í' | 'Ì' => ('I', NONE),
        'Ô' | 'Ö' | 'Ó' | 'Ò' => ('O', NONE),
        'Ù' | 'Û' | 'Ü' | 'Ú' => ('U', NONE),
        'Ñ' => ('N', NONE),
        '’' | '‘' => ('\'', NONE),
        '“' | '”' | '«' | '»' => ('"', NONE),
        '–' | '—' => ('-', NONE),
        '…' => ('.', NONE),
        _ => (c, NONE),
    }
}

// Minimalist 5x7 ASCII font (Characters 32 to 126)
// Each character is 5 bytes (each byte is 1 column of 7 vertical pixels, bit 0 = top).
static FONT_5X7: [[u8; 5]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // Space
    [0x00, 0x00, 0x5F, 0x00, 0x00], // !
    [0x00, 0x07, 0x00, 0x07, 0x00], // "
    [0x14, 0x7F, 0x14, 0x7F, 0x14], // #
    [0x24, 0x2A, 0x7F, 0x2A, 0x12], // $
    [0x23, 0x13, 0x08, 0x64, 0x62], // %
    [0x36, 0x49, 0x55, 0x22, 0x50], // &
    [0x00, 0x05, 0x03, 0x00, 0x00], // '
    [0x00, 0x1C, 0x22, 0x41, 0x00], // (
    [0x00, 0x41, 0x22, 0x1C, 0x00], // )
    [0x14, 0x08, 0x3E, 0x08, 0x14], // *
    [0x08, 0x08, 0x3E, 0x08, 0x08], // +
    [0x00, 0x50, 0x30, 0x00, 0x00], // ,
    [0x08, 0x08, 0x08, 0x08, 0x08], // -
    [0x00, 0x60, 0x60, 0x00, 0x00], // .
    [0x20, 0x10, 0x08, 0x04, 0x02], // /
    [0x3E, 0x51, 0x49, 0x45, 0x3E], // 0
    [0x00, 0x42, 0x7F, 0x40, 0x00], // 1
    [0x42, 0x61, 0x51, 0x49, 0x46], // 2
    [0x21, 0x41, 0x45, 0x4B, 0x31], // 3
    [0x18, 0x14, 0x12, 0x7F, 0x10], // 4
    [0x27, 0x45, 0x45, 0x45, 0x39], // 5
    [0x3C, 0x4A, 0x49, 0x49, 0x30], // 6
    [0x01, 0x71, 0x09, 0x05, 0x03], // 7
    [0x36, 0x49, 0x49, 0x49, 0x36], // 8
    [0x06, 0x49, 0x49, 0x29, 0x1E], // 9
    [0x00, 0x36, 0x36, 0x00, 0x00], // :
    [0x00, 0x56, 0x36, 0x00, 0x00], // ;
    [0x08, 0x14, 0x22, 0x41, 0x00], // <
    [0x14, 0x14, 0x14, 0x14, 0x14], // =
    [0x00, 0x41, 0x22, 0x14, 0x08], // >
    [0x02, 0x01, 0x51, 0x09, 0x06], // ?
    [0x32, 0x49, 0x79, 0x41, 0x3E], // @
    [0x7E, 0x11, 0x11, 0x11, 0x7E], // A
    [0x7F, 0x49, 0x49, 0x49, 0x36], // B
    [0x3E, 0x41, 0x41, 0x41, 0x22], // C
    [0x7F, 0x41, 0x41, 0x22, 0x1C], // D
    [0x7F, 0x49, 0x49, 0x49, 0x41], // E
    [0x7F, 0x09, 0x09, 0x09, 0x01], // F
    [0x3E, 0x41, 0x49, 0x49, 0x7A], // G
    [0x7F, 0x08, 0x08, 0x08, 0x7F], // H
    [0x00, 0x41, 0x7F, 0x41, 0x00], // I
    [0x20, 0x40, 0x41, 0x3F, 0x01], // J
    [0x7F, 0x08, 0x14, 0x22, 0x41], // K
    [0x7F, 0x40, 0x40, 0x40, 0x40], // L
    [0x7F, 0x02, 0x0C, 0x02, 0x7F], // M
    [0x7F, 0x04, 0x08, 0x10, 0x7F], // N
    [0x3E, 0x41, 0x41, 0x41, 0x3E], // O
    [0x7F, 0x09, 0x09, 0x09, 0x06], // P
    [0x3E, 0x41, 0x51, 0x21, 0x5E], // Q
    [0x7F, 0x09, 0x19, 0x29, 0x46], // R
    [0x46, 0x49, 0x49, 0x49, 0x31], // S
    [0x01, 0x01, 0x7F, 0x01, 0x01], // T
    [0x3F, 0x40, 0x40, 0x40, 0x3F], // U
    [0x1F, 0x20, 0x40, 0x20, 0x1F], // V
    [0x3F, 0x40, 0x38, 0x40, 0x3F], // W
    [0x63, 0x14, 0x08, 0x14, 0x63], // X
    [0x07, 0x08, 0x70, 0x08, 0x07], // Y
    [0x61, 0x51, 0x49, 0x45, 0x43], // Z
    [0x00, 0x7F, 0x41, 0x41, 0x00], // [
    [0x02, 0x04, 0x08, 0x10, 0x20], // \
    [0x00, 0x41, 0x41, 0x7F, 0x00], // ]
    [0x04, 0x02, 0x01, 0x02, 0x04], // ^
    [0x40, 0x40, 0x40, 0x40, 0x40], // _
    [0x00, 0x01, 0x02, 0x04, 0x00], // `
    [0x20, 0x54, 0x54, 0x54, 0x78], // a
    [0x7F, 0x48, 0x44, 0x44, 0x38], // b
    [0x38, 0x44, 0x44, 0x44, 0x20], // c
    [0x38, 0x44, 0x44, 0x48, 0x7F], // d
    [0x38, 0x54, 0x54, 0x54, 0x18], // e
    [0x08, 0x7E, 0x09, 0x01, 0x02], // f
    [0x0C, 0x52, 0x52, 0x52, 0x3E], // g
    [0x7F, 0x08, 0x04, 0x04, 0x78], // h
    [0x00, 0x44, 0x7D, 0x40, 0x00], // i
    [0x20, 0x40, 0x44, 0x3D, 0x00], // j
    [0x7F, 0x10, 0x28, 0x44, 0x00], // k
    [0x00, 0x41, 0x7F, 0x40, 0x00], // l
    [0x7C, 0x04, 0x18, 0x04, 0x78], // m
    [0x7C, 0x08, 0x04, 0x04, 0x78], // n
    [0x38, 0x44, 0x44, 0x44, 0x38], // o
    [0x7C, 0x14, 0x14, 0x14, 0x08], // p
    [0x08, 0x14, 0x14, 0x18, 0x7C], // q
    [0x7C, 0x08, 0x04, 0x04, 0x08], // r
    [0x48, 0x54, 0x54, 0x54, 0x20], // s
    [0x04, 0x3F, 0x44, 0x40, 0x20], // t
    [0x3C, 0x40, 0x40, 0x20, 0x7C], // u
    [0x1C, 0x20, 0x40, 0x20, 0x1C], // v
    [0x3C, 0x40, 0x30, 0x40, 0x3C], // w
    [0x44, 0x28, 0x10, 0x28, 0x44], // x
    [0x0C, 0x50, 0x50, 0x50, 0x3C], // y
    [0x44, 0x64, 0x54, 0x4C, 0x44], // z
    [0x00, 0x08, 0x36, 0x41, 0x00], // {
    [0x00, 0x00, 0x7F, 0x00, 0x00], // |
    [0x00, 0x41, 0x36, 0x08, 0x00], // }
    [0x08, 0x08, 0x2A, 0x1C, 0x08], // ~
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_are_drawn_over_the_base_letter() {
        let mut a = G13LcdBuffer::new();
        a.draw_char(0, 0, 'e');
        let mut b = G13LcdBuffer::new();
        b.draw_char(0, 0, 'é');
        assert!(b.get_pixel(3, 0) && b.get_pixel(2, 1), "acute accent");
        for x in 0..5 {
            for y in 2..7 {
                assert_eq!(a.get_pixel(x, y), b.get_pixel(x, y), "same letter body");
            }
        }
    }

    #[test]
    fn scaled_text_and_width() {
        let mut b = G13LcdBuffer::new();
        b.draw_text_scaled(0, 0, "1", 3);
        assert_eq!(G13LcdBuffer::text_width("12:34", 3), (5 * 6 - 1) * 3);
        // '1' at scale 3: its stem (column 2 of the glyph) spans x 6..9
        assert!(b.get_pixel(6, 3) && b.get_pixel(8, 20));
    }

    #[test]
    fn text_window_scrolls_and_clips() {
        let mut full = G13LcdBuffer::new();
        full.draw_text(0, 0, "ABCDEFGH");
        let mut win = G13LcdBuffer::new();
        win.draw_text_window(10, 5, 20, 6, "ABCDEFGH"); // starts at "B", 20 px wide
        for x in 0..20 {
            for y in 0..7 {
                assert_eq!(win.get_pixel(10 + x, 5 + y), full.get_pixel(6 + x, y), "x={x} y={y}");
            }
        }
        assert!(!win.get_pixel(30, 5) && !win.get_pixel(9, 6));
    }

    #[test]
    fn undrawable_characters_leave_no_gap() {
        let mut a = G13LcdBuffer::new();
        a.draw_text(0, 0, "Wardog Wipe");
        let mut b = G13LcdBuffer::new();
        b.draw_text(0, 0, "🌇Wardog Wipe🕹\u{fe0f}");
        assert_eq!(a, b);
        assert_eq!(G13LcdBuffer::text_width("🌇ab", 1), G13LcdBuffer::text_width("ab", 1));
    }

    #[test]
    fn bars_and_inversion() {
        let mut b = G13LcdBuffer::new();
        b.draw_bar(0, 0, 44, 8, 0.5);
        assert!(b.get_pixel(2, 3) && b.get_pixel(21, 3) && !b.get_pixel(23, 3));
        b.invert_rect(0, 0, 160, 43);
        assert!(!b.get_pixel(2, 3) && b.get_pixel(23, 3));
        assert_eq!(b.pixels().len(), LCD_BUFFER_SIZE);
    }
}

