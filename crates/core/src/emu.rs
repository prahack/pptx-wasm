//! English Metric Units — the unit the whole presentation model is expressed in.
//!
//! 914400 EMU = 1 inch = 72 pt, so 12700 EMU = 1 pt exactly. Keeping the model in
//! integer EMUs means no accumulated float drift through the inheritance chain; the
//! single conversion to `f32` points happens in layout.

/// EMUs per inch.
pub const PER_INCH: i64 = 914_400;
/// EMUs per printer's point (1/72 inch).
pub const PER_POINT: i64 = 12_700;
/// EMUs per centimetre.
pub const PER_CM: i64 = 360_000;

/// A length in EMUs.
pub type Emu = i64;

/// EMU → points. The only sanctioned way out of EMU space.
#[inline]
pub fn to_pt(emu: Emu) -> f32 {
    emu as f32 / PER_POINT as f32
}

/// Points → EMUs, rounding to nearest.
#[inline]
pub fn from_pt(pt: f32) -> Emu {
    (pt * PER_POINT as f32).round() as Emu
}

#[inline]
pub fn from_inches(inches: f32) -> Emu {
    (inches * PER_INCH as f32).round() as Emu
}

/// A point in EMU space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmuPoint {
    pub x: Emu,
    pub y: Emu,
}

/// An axis-aligned rectangle in EMU space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmuRect {
    pub x: Emu,
    pub y: Emu,
    pub w: Emu,
    pub h: Emu,
}

impl EmuRect {
    pub const fn new(x: Emu, y: Emu, w: Emu, h: Emu) -> Self {
        Self { x, y, w, h }
    }
    #[inline]
    pub fn right(&self) -> Emu {
        self.x + self.w
    }
    #[inline]
    pub fn bottom(&self) -> Emu {
        self.y + self.h
    }
    #[inline]
    pub fn center(&self) -> EmuPoint {
        EmuPoint {
            x: self.x + self.w / 2,
            y: self.y + self.h / 2,
        }
    }
}

/// OOXML angles are in 60000ths of a degree.
pub const ANGLE_SCALE: f32 = 60_000.0;

/// `<a:xfrm rot="...">` → degrees.
#[inline]
pub fn angle_to_degrees(raw: i32) -> f32 {
    raw as f32 / ANGLE_SCALE
}

/// `<a:xfrm rot="...">` → radians.
#[inline]
pub fn angle_to_radians(raw: i32) -> f32 {
    angle_to_degrees(raw).to_radians()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_conversion_is_exact_at_integer_points() {
        assert_eq!(to_pt(PER_POINT), 1.0);
        assert_eq!(to_pt(PER_INCH), 72.0);
        assert_eq!(from_pt(18.0), 228_600);
    }

    #[test]
    fn standard_slide_size_is_13_333_by_7_5_inches() {
        // 16:9 default from PowerPoint.
        assert_eq!(to_pt(12_192_000), 960.0);
        assert_eq!(to_pt(6_858_000), 540.0);
    }

    #[test]
    fn angles_scale_by_sixty_thousand() {
        assert_eq!(angle_to_degrees(2_700_000), 45.0);
        assert_eq!(angle_to_degrees(-5_400_000), -90.0);
    }
}
