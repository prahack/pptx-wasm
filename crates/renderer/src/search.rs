//! Finding text on a laid-out slide, with the rectangles to highlight it.
//!
//! `plain_text()` already tells you *whether* a slide contains a word. That is enough to
//! filter a deck and useless for showing anyone where the word is, which is what a reader
//! actually wants — a viewer that can find but not point is barely better than one that
//! cannot find.
//!
//! Everything here works off [`crate::textlayer::PositionedText`], so a match is placed by
//! the same walk that painted the glyphs and carries per-character advances from the same
//! measurement. No second layout pass, and nothing that can disagree with the canvas.

use pptx_core::dl::Rect;

use crate::textlayer::PositionedText;

/// One occurrence of the query on a slide.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// The matched text as it appears on the slide, which for a case-insensitive search
    /// is not necessarily the query.
    pub text: String,
    /// Where to draw the highlight, in device pixels.
    ///
    /// More than one when the match spans several runs — "the **bold** word" is three
    /// runs, and a match across them needs a rectangle per run rather than one box that
    /// swallows the gaps.
    pub rects: Vec<Rect>,
}

/// Where a character sits in the concatenated slide text.
struct Span {
    run: usize,
    /// Index of the first `char` of this run within the joined string.
    start: usize,
}

/// Joins the runs into one searchable string.
///
/// Runs on the same baseline are concatenated directly, so a phrase broken across a
/// formatting change still matches. Runs on different baselines are separated by a
/// newline, so a phrase cannot match across a line break — the words are not adjacent on
/// screen, and highlighting them as though they were would draw a box across unrelated
/// text.
fn join(runs: &[PositionedText]) -> (String, Vec<Span>) {
    let mut joined = String::new();
    let mut spans = Vec::with_capacity(runs.len());
    let mut prev_baseline: Option<f32> = None;
    let mut chars = 0usize;
    for (i, run) in runs.iter().enumerate() {
        if let Some(y) = prev_baseline {
            if (run.y - y).abs() > 0.5 {
                joined.push('\n');
                chars += 1;
            }
        }
        spans.push(Span {
            run: i,
            start: chars,
        });
        joined.push_str(&run.text);
        chars += run.text.chars().count();
        prev_baseline = Some(run.y);
    }
    (joined, spans)
}

/// The x offset and width of `from..to` within a run, in device pixels.
fn slice_x(run: &PositionedText, from: usize, to: usize) -> (f32, f32) {
    let total = run.text.chars().count();
    if total == 0 || to <= from {
        return (0.0, 0.0);
    }
    if run.advances.len() == total {
        let x: f32 = run.advances.iter().take(from).sum();
        let w: f32 = run.advances.iter().take(to).skip(from).sum();
        (x, w)
    } else {
        // No per-character measurement available; the run's own width is still exact, so
        // divide it evenly. Wrong for proportional faces, but bounded by the run.
        let per = run.width / total as f32;
        (per * from as f32, per * (to - from) as f32)
    }
}

/// Finds every occurrence of `needle` among the runs.
///
/// Matching is on `char` boundaries rather than bytes, so a query is never cut through the
/// middle of a multi-byte character. An empty needle matches nothing, because the
/// alternative is a match between every pair of characters.
pub fn find(runs: &[PositionedText], needle: &str, case_sensitive: bool) -> Vec<Match> {
    if needle.is_empty() || runs.is_empty() {
        return Vec::new();
    }
    let (joined, spans) = join(runs);
    let hay: Vec<char> = if case_sensitive {
        joined.chars().collect()
    } else {
        joined.chars().flat_map(|c| c.to_lowercase()).collect()
    };
    let pat: Vec<char> = if case_sensitive {
        needle.chars().collect()
    } else {
        needle.chars().flat_map(|c| c.to_lowercase()).collect()
    };
    // Lowercasing can change the character count for a few scripts, which would put every
    // offset out by however much it changed. Fall back to a case-sensitive scan rather
    // than report confidently wrong rectangles.
    let aligned = case_sensitive || hay.len() == joined.chars().count();
    let (hay, pat) = if aligned {
        (hay, pat)
    } else {
        (joined.chars().collect(), needle.chars().collect())
    };
    if pat.is_empty() || pat.len() > hay.len() {
        return Vec::new();
    }

    let original: Vec<char> = joined.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + pat.len() <= hay.len() {
        if hay[i..i + pat.len()] != pat[..] {
            i += 1;
            continue;
        }
        let (start, end) = (i, i + pat.len());
        let mut rects = Vec::new();
        for span in &spans {
            let run = &runs[span.run];
            let len = run.text.chars().count();
            let (rs, re) = (span.start, span.start + len);
            let (from, to) = (start.max(rs), end.min(re));
            if from >= to {
                continue;
            }
            let (dx, w) = slice_x(run, from - rs, to - rs);
            // `y` is the baseline; the box is one em tall and sits on it, which is close
            // enough to a highlight and needs no font metrics we do not have.
            rects.push(Rect::new(run.x + dx, run.y - run.size, w, run.size));
        }
        out.push(Match {
            text: original[start..end].iter().collect(),
            rects,
        });
        // Overlapping matches are not reported: "aa" in "aaa" is one occurrence to a
        // reader, and highlighting two overlapping boxes looks like a rendering fault.
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, x: f32, y: f32, width: f32) -> PositionedText {
        let n = text.chars().count().max(1);
        PositionedText {
            text: text.into(),
            x,
            y,
            width,
            size: 10.0,
            family: "Calibri".into(),
            weight: 400,
            italic: false,
            rotation: 0.0,
            advances: vec![width / n as f32; text.chars().count()],
        }
    }

    #[test]
    fn a_match_inside_one_run_is_located_within_it() {
        let runs = [run("Revenue up 12%", 100.0, 50.0, 140.0)];
        let m = find(&runs, "up", true);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, "up");
        assert_eq!(m[0].rects.len(), 1);
        // "up" starts at character 8 of 14, each 10px wide.
        assert!(
            (m[0].rects[0].x - 180.0).abs() < 0.01,
            "{:?}",
            m[0].rects[0]
        );
        assert!((m[0].rects[0].w - 20.0).abs() < 0.01, "{:?}", m[0].rects[0]);
    }

    #[test]
    fn a_match_spanning_runs_gets_one_rect_per_run() {
        // "the bold word" split by a formatting change, all on one baseline.
        let runs = [
            run("the ", 0.0, 20.0, 40.0),
            run("bold", 40.0, 20.0, 40.0),
            run(" word", 80.0, 20.0, 50.0),
        ];
        let m = find(&runs, "bold word", true);
        assert_eq!(m.len(), 1);
        // One box spanning the gap would cover the whole line; two is correct.
        assert_eq!(m[0].rects.len(), 2, "{:?}", m[0].rects);
        assert!((m[0].rects[0].x - 40.0).abs() < 0.01);
        assert!((m[0].rects[1].x - 80.0).abs() < 0.01);
    }

    #[test]
    fn a_phrase_cannot_match_across_a_line_break() {
        // Same words, but the second run is on the next line. They are not adjacent on
        // screen, so highlighting them as one phrase would box unrelated text.
        let runs = [run("bold", 0.0, 20.0, 40.0), run("word", 0.0, 40.0, 40.0)];
        assert!(find(&runs, "boldword", true).is_empty());
        assert_eq!(find(&runs, "bold", true).len(), 1);
    }

    #[test]
    fn case_insensitive_reports_the_text_as_it_appears() {
        let runs = [run("Quarterly Revenue", 0.0, 20.0, 170.0)];
        let m = find(&runs, "REVENUE", false);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, "Revenue", "the slide's casing, not the query's");
    }

    #[test]
    fn overlapping_occurrences_are_reported_once() {
        let runs = [run("aaa", 0.0, 20.0, 30.0)];
        assert_eq!(find(&runs, "aa", true).len(), 1);
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        let runs = [run("anything", 0.0, 20.0, 80.0)];
        assert!(find(&runs, "", false).is_empty());
    }

    #[test]
    fn a_run_without_advances_still_produces_a_bounded_rect() {
        // No measurer means no per-character widths. The rect must stay inside the run
        // rather than be dropped or run off the end of it.
        let mut r = run("Revenue", 100.0, 50.0, 70.0);
        r.advances.clear();
        let m = find(&[r], "venue", true);
        assert_eq!(m.len(), 1);
        let rect = m[0].rects[0];
        assert!(rect.x >= 100.0 && rect.right() <= 170.0 + 0.01, "{rect:?}");
    }
}
