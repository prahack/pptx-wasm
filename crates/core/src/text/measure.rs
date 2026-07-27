use crate::dl::FontSpec;

use super::metrics::FontMetrics;

/// The result of measuring one string in one font.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeasuredRun {
    /// Total advance width in points.
    pub width: f32,
    /// Per-`char` advance in points, parallel to `text.chars()`.
    ///
    /// For scripts where a cluster spans several `char`s (combining marks, emoji
    /// sequences) the cluster's whole advance sits on its first `char` and the rest are
    /// zero. Line breaking only ever sums contiguous ranges, so that convention keeps
    /// break positions correct without the caller needing cluster awareness.
    pub advances: Vec<f32>,
}

impl MeasuredRun {
    /// Advance width of `chars[start..end]`, in points.
    pub fn width_of_range(&self, start: usize, end: usize) -> f32 {
        let end = end.min(self.advances.len());
        if start >= end {
            return 0.0;
        }
        self.advances
            .get(start..end)
            .map_or(0.0, |s| s.iter().sum())
    }

    /// Largest `n` such that the first `n` chars fit in `max_width`.
    pub fn chars_fitting(&self, max_width: f32) -> usize {
        let mut acc = 0.0;
        for (i, a) in self.advances.iter().enumerate() {
            if acc + a > max_width {
                return i;
            }
            acc += a;
        }
        self.advances.len()
    }
}

/// Supplies the font metrics layout needs. Implementations must be deterministic for a
/// given (text, font) pair within a session — layout caches aggressively and assumes a
/// repeat measurement cannot change a wrap point.
pub trait TextMeasure {
    /// Advance widths for `text` in `font`.
    fn measure(&self, text: &str, font: &FontSpec) -> MeasuredRun;

    /// Vertical metrics for `font`, independent of any particular string.
    fn font_metrics(&self, font: &FontSpec) -> FontMetrics;

    /// Whether `family` resolves to a real installed face. Used to decide when to walk
    /// the fallback chain and to report missing fonts to the host application.
    ///
    /// The default answer is "yes": an implementation that cannot tell should not cause
    /// layout to substitute fonts it did not need to.
    fn has_family(&self, _family: &str) -> bool {
        true
    }
}

impl<T: TextMeasure + ?Sized> TextMeasure for &T {
    fn measure(&self, text: &str, font: &FontSpec) -> MeasuredRun {
        (**self).measure(text, font)
    }
    fn font_metrics(&self, font: &FontSpec) -> FontMetrics {
        (**self).font_metrics(font)
    }
    fn has_family(&self, family: &str) -> bool {
        (**self).has_family(family)
    }
}

impl<T: TextMeasure + ?Sized> TextMeasure for Box<T> {
    fn measure(&self, text: &str, font: &FontSpec) -> MeasuredRun {
        (**self).measure(text, font)
    }
    fn font_metrics(&self, font: &FontSpec) -> FontMetrics {
        (**self).font_metrics(font)
    }
    fn has_family(&self, family: &str) -> bool {
        (**self).has_family(family)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(advances: &[f32]) -> MeasuredRun {
        MeasuredRun {
            width: advances.iter().sum(),
            advances: advances.to_vec(),
        }
    }

    #[test]
    fn range_widths_sum_the_right_slice() {
        let r = run(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(r.width_of_range(0, 4), 10.0);
        assert_eq!(r.width_of_range(1, 3), 5.0);
        assert_eq!(r.width_of_range(3, 3), 0.0);
    }

    #[test]
    fn range_widths_clamp_instead_of_panicking() {
        let r = run(&[1.0, 2.0]);
        assert_eq!(r.width_of_range(0, 99), 3.0);
        assert_eq!(r.width_of_range(5, 9), 0.0);
    }

    #[test]
    fn chars_fitting_stops_before_the_char_that_overflows() {
        let r = run(&[10.0, 10.0, 10.0]);
        assert_eq!(r.chars_fitting(25.0), 2);
        assert_eq!(r.chars_fitting(30.0), 3);
        assert_eq!(r.chars_fitting(100.0), 3);
        assert_eq!(r.chars_fitting(5.0), 0);
    }
}
