//! A bitmap face's metrics: where each glyph sits in the atlas page and how to
//! lay it out. Pure data and measuring, so the chain loader, which draws its
//! one label from the same atlas, includes this file with `#[path]` too.

/// One glyph's place in the atlas page and how to lay it out.
#[derive(Clone, Copy)]
pub struct Glyph {
    pub u: u8,
    pub v: u8,
    pub w: u8,
    pub h: u8,
    pub xoff: i8,
    pub yoff: i8,
    pub adv: u8,
}

/// Printable ASCII, space to tilde.
#[allow(dead_code)]
pub struct Face {
    pub ascent: i32,
    pub line: i32,
    pub glyphs: [Glyph; 95],
}

impl Face {
    /// Rows from a glyph's draw position down to the baseline row (the
    /// bottom row of `H`), for lining faces of different sizes up on one baseline.
    #[allow(dead_code)]
    pub fn baseline(&self) -> i32 {
        let h = self.glyph(b'H');
        h.yoff as i32 + h.h as i32 - 1
    }

    pub fn glyph(&self, c: u8) -> &Glyph {
        let i = if (32..=126).contains(&c) {
            c - 32
        } else {
            b'?' - 32
        };
        &self.glyphs[i as usize]
    }

    /// Width in pixels of `s` with `track` extra pixels after every glyph.
    pub fn width(&self, s: &str, track: i32) -> i32 {
        if s.is_empty() {
            return 0;
        }
        s.bytes()
            .map(|c| self.glyph(c).adv as i32 + track)
            .sum::<i32>()
            - track
    }

    /// Rows from a glyph's draw position down to the middle row of the capital
    /// letters (taken from `H`). Aligning on this row, rather than on the
    /// baseline or the line box, puts faces of different sizes on one centre
    /// line.
    pub fn cap_mid(&self) -> i32 {
        let h = self.glyph(b'H');
        h.yoff as i32 + (h.h as i32 - 1) / 2
    }
}
