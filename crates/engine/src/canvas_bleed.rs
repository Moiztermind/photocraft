//! Canvas Size › Canvas extension "Edge Pixels" / "Mirror": fills the area a canvas grows by from
//! the image's own edges, so a print can bleed past its trim line without a white border.

use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeFill {
    /// The outermost row or column repeats outward.
    Stretch,
    /// The image reflects about its edge (the edge pixel itself is not doubled).
    Mirror,
}

impl EdgeFill {
    pub(crate) fn from_id(s: &str) -> Option<Self> {
        match s {
            "edge" => Some(Self::Stretch),
            "mirror" => Some(Self::Mirror),
            _ => None,
        }
    }

    /// Source index (0..n) for position `t` relative to the old image's edge.
    fn map(self, t: i64, n: i64) -> i64 {
        if n <= 1 {
            return 0;
        }
        match self {
            Self::Stretch => t.clamp(0, n - 1),
            Self::Mirror => {
                let p = 2 * (n - 1);
                let m = t.rem_euclid(p);
                if m >= n { p - m } else { m }
            }
        }
    }
}

/// Fills the part of `canvas` outside `old` (where the image was before the resize, in new
/// document coordinates) on every pixel layer and layer mask. Pixels a layer already has there
/// (content that hung past the old canvas) are kept; only empty ones are filled.
pub(crate) fn extend_edges(layers: &mut [Layer], old: Rect, canvas: Rect, mode: EdgeFill) {
    if old.is_empty() || canvas.is_empty() {
        return;
    }
    for l in layers {
        if let Some(m) = &mut l.mask {
            extend_surface(&mut m.surface, old, canvas, mode);
        }
        match &mut l.content {
            LayerContent::Raster(s) => extend_surface(s, old, canvas, mode),
            LayerContent::Group(g) => extend_edges(&mut g.children, old, canvas, mode),
            _ => {}
        }
    }
}

fn extend_surface(surf: &mut Surface, old: Rect, canvas: Rect, mode: EdgeFill) {
    // The four margins: top and bottom span the full new width (corners included), left and
    // right only the rows in between.
    let mid_y0 = old.y0.max(canvas.y0);
    let mid_y1 = old.y1.min(canvas.y1);
    let margins = [
        Rect::new(canvas.x0, canvas.y0, canvas.x1, old.y0.min(canvas.y1)),
        Rect::new(canvas.x0, old.y1.max(canvas.y0), canvas.x1, canvas.y1),
        Rect::new(canvas.x0, mid_y0, old.x0.min(canvas.x1), mid_y1),
        Rect::new(old.x1.max(canvas.x0), mid_y0, canvas.x1, mid_y1),
    ];
    let bpp = surf.format().bytes_per_pixel();
    let (ow, oh) = (i64::from(old.width()), i64::from(old.height()));
    for m in margins {
        if m.is_empty() {
            continue;
        }
        // Only the band of the old image the margin can reach is read: as deep as the margin
        // (plus one for the mirror's pivot), never deeper than the image.
        let src = source_band(old, m);
        let band = surf.to_interleaved(src);
        let mut out = surf.to_interleaved(m);
        let default = surf.default_bytes().to_vec();
        let (bw, mw) = (src.width() as usize, m.width() as usize);
        for y in m.y0..m.y1 {
            let sy = mode.map(i64::from(y) - i64::from(old.y0), oh) + i64::from(old.y0) - i64::from(src.y0);
            for x in m.x0..m.x1 {
                let sx = mode.map(i64::from(x) - i64::from(old.x0), ow) + i64::from(old.x0) - i64::from(src.x0);
                let o = ((y - m.y0) as usize * mw + (x - m.x0) as usize) * bpp;
                let Some(dst) = out.get_mut(o..o + bpp) else { continue };
                if dst != default.as_slice() {
                    continue;
                }
                let (Ok(sx), Ok(sy)) = (usize::try_from(sx), usize::try_from(sy)) else { continue };
                let s = (sy * bw + sx) * bpp;
                if let Some(px) = band.get(s..s + bpp) {
                    dst.copy_from_slice(px);
                }
            }
        }
        surf.write_interleaved(m, &out);
    }
}

/// The part of `old` that pixels of margin `m` map to: the full old width or height across,
/// and from the touching edge inward as deep as the margin is (plus one).
fn source_band(old: Rect, m: Rect) -> Rect {
    let depth = |len: u32, n: u32| (len.saturating_add(1)).min(n) as i32;
    let (mut x0, mut x1, mut y0, mut y1) = (old.x0, old.x1, old.y0, old.y1);
    if m.y1 <= old.y0 {
        y1 = old.y0 + depth(m.height(), old.height());
    } else if m.y0 >= old.y1 {
        y0 = old.y1 - depth(m.height(), old.height());
    } else if m.x1 <= old.x0 {
        x1 = old.x0 + depth(m.width(), old.width());
    } else if m.x0 >= old.x1 {
        x0 = old.x1 - depth(m.width(), old.width());
    }
    Rect::new(x0, y0, x1, y1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_clamps_and_mirror_reflects_without_doubling_the_edge() {
        let n = 4; // pixels 0 1 2 3
        let s: Vec<i64> = (-3..7).map(|t| EdgeFill::Stretch.map(t, n)).collect();
        assert_eq!(s, [0, 0, 0, 0, 1, 2, 3, 3, 3, 3]);
        let m: Vec<i64> = (-3..7).map(|t| EdgeFill::Mirror.map(t, n)).collect();
        assert_eq!(m, [3, 2, 1, 0, 1, 2, 3, 2, 1, 0]);
        // Wider than the image: keeps reflecting back and forth.
        assert_eq!(EdgeFill::Mirror.map(-7, n), 1);
        assert_eq!(EdgeFill::Mirror.map(9, 1), 0);
        assert_eq!(EdgeFill::Stretch.map(-5, 0), 0);
    }

    #[test]
    fn source_band_reaches_only_as_deep_as_the_margin() {
        let old = Rect::new(10, 10, 110, 60);
        assert_eq!(source_band(old, Rect::new(0, 0, 120, 10)), Rect::new(10, 10, 110, 21));
        assert_eq!(source_band(old, Rect::new(0, 60, 120, 70)), Rect::new(10, 49, 110, 60));
        assert_eq!(source_band(old, Rect::new(0, 10, 10, 60)), Rect::new(10, 10, 21, 60));
        assert_eq!(source_band(old, Rect::new(110, 10, 120, 60)), Rect::new(99, 10, 110, 60));
        // A margin deeper than the image reads the whole image.
        let tall = Rect::new(10, 300, 110, 350);
        assert_eq!(source_band(tall, Rect::new(0, 0, 120, 300)), tall);
    }
}
