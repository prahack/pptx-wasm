//! Backend B: WebGPU. **Stub.**
//!
//! This exists to keep Spike B's conclusion honest. The point of the [`Renderer`] trait
//! is that a GPU backend can be dropped in without touching layout, and the only way to
//! know that is true is to write down exactly what such a backend needs from the display
//! list — and to check, at compile time, that the display list can supply it.
//!
//! [`Requirements::analyse`] does that check. It walks a real display list and reports
//! what a GPU backend would have to build: how many paths need tessellating, how many
//! distinct glyph runs need atlas space, whether any command needs a feature the display
//! list cannot yet express. If a change upstream ever makes a display list un-renderable
//! on the GPU — an effect that only Canvas2D can do, say — the test in this file fails
//! *before* anyone has invested in the backend.
//!
//! ## What shipping this backend requires
//!
//! 1. **Path tessellation.** Fill and stroke paths must become triangles. `lyon` is the
//!    obvious choice; the display list's verb/point representation feeds it directly.
//! 2. **A glyph atlas.** [`TextRun::advances`] already carries per-character positions, so
//!    the backend does not need a shaper — but it does need rasterised glyphs, which means
//!    either `swash`/`fontdue` over font bytes, or `OffscreenCanvas` rasterisation per
//!    glyph. The first is the deterministic path; the second reuses browser fonts.
//! 3. **Clip handling.** Rectangular clips are scissor rects; path clips need a stencil
//!    buffer. Both nest, so the state stack becomes a real stack of scissor/stencil state.
//! 4. **Gradients.** Either as a fragment-shader uniform or a 1D lookup texture.
//!
//! None of that blocks shipping: Canvas2D is the default and satisfies the same golden
//! tests. This backend is the optimisation, not the product.

use pptx_core::dl::{Command, DisplayList, Paint, PathVerb, Transform};

use crate::Renderer;

/// What a GPU backend would need to draw a given display list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Requirements {
    /// Paths needing fill tessellation.
    pub fill_paths: usize,
    /// Paths needing stroke tessellation (expansion to a triangle strip).
    pub stroke_paths: usize,
    /// Total path segments, which bounds the tessellation work.
    pub path_segments: usize,
    /// Distinct (font, character) pairs needing atlas space.
    pub glyphs: usize,
    /// Text runs, i.e. draw calls if glyphs are batched per run.
    pub text_runs: usize,
    /// Images needing GPU textures.
    pub textures: usize,
    /// Clips that are plain rectangles and can use a scissor rect.
    pub scissor_clips: usize,
    /// Clips needing a stencil buffer.
    pub stencil_clips: usize,
    /// Gradients needing a shader or lookup texture.
    pub gradients: usize,
    /// Shadows, each of which needs an offscreen render target plus a separable blur
    /// pass. By far the most expensive primitive here, which is why it is counted apart
    /// from everything else.
    pub offscreen_passes: usize,
    /// Hatch fills, each needing a small tile rasterised once and then sampled with
    /// repeat addressing.
    pub generated_tiles: usize,
    /// Commands this backend could not express. Non-empty means the abstraction has
    /// sprung a leak and something upstream is assuming Canvas2D.
    pub unsupported: Vec<String>,
}

impl Requirements {
    /// Walks a display list and reports what a GPU backend would need for it.
    pub fn analyse(dl: &DisplayList) -> Requirements {
        let mut r = Requirements::default();
        let mut glyph_keys: Vec<(String, char)> = Vec::new();
        let mut textures: Vec<u32> = Vec::new();

        for cmd in &dl.commands {
            match cmd {
                Command::Save | Command::Restore | Command::Concat(_) => {}
                Command::ClipRect(_) => r.scissor_clips += 1,
                Command::SetShadow(shadow) => {
                    if shadow.is_some() {
                        r.offscreen_passes += 1;
                    }
                }
                // The contents go to a render target, the target's alpha is faded inward
                // from its own silhouette, and the result is composited back. Costed the
                // same way as a shadow: one extra pass. It is expressible on a GPU — a
                // distance-to-edge falloff over the shape's coverage — so it is a cost,
                // not something the backend would have to refuse.
                Command::BeginSoftEdge(_) => r.offscreen_passes += 1,
                Command::EndSoftEdge => {}
                Command::ClipPath { path, .. } => {
                    r.stencil_clips += 1;
                    r.path_segments += path.verbs.len();
                }
                Command::FillPath { path, paint, .. } => {
                    r.fill_paths += 1;
                    r.path_segments += path.verbs.len();
                    r.note_paint(paint, &mut textures);
                }
                Command::StrokePath { path, stroke } => {
                    r.stroke_paths += 1;
                    // Curves need flattening before expansion, so they cost more.
                    r.path_segments += path
                        .verbs
                        .iter()
                        .map(|v| match v {
                            PathVerb::QuadTo | PathVerb::CubicTo => 8,
                            _ => 1,
                        })
                        .sum::<usize>();
                    r.note_paint(&stroke.paint, &mut textures);
                }
                Command::DrawImage { image, .. } => {
                    if !textures.contains(&image.0) {
                        textures.push(image.0);
                    }
                }
                Command::DrawText(run) => {
                    r.text_runs += 1;
                    let key = run.font.to_css();
                    for ch in run.text.chars() {
                        if ch.is_whitespace() {
                            continue;
                        }
                        let pair = (key.clone(), ch);
                        if !glyph_keys.contains(&pair) {
                            glyph_keys.push(pair);
                        }
                    }
                    // Without advances the backend has no way to position glyphs, and
                    // would need its own shaper — a genuine gap, so it is recorded.
                    if !run.has_glyph_positions() && !run.text.trim().is_empty() {
                        r.unsupported
                            .push(format!("text run {:?} has no glyph advances", run.text));
                    }
                }
            }
        }
        r.glyphs = glyph_keys.len();
        r.textures = textures.len();
        r
    }

    fn note_paint(&mut self, paint: &Paint, textures: &mut Vec<u32>) {
        match paint {
            Paint::Solid(_) => {}
            Paint::Gradient(_) => self.gradients += 1,
            Paint::Image { image, .. } => {
                if !textures.contains(&image.0) {
                    textures.push(image.0);
                }
            }
            // A hatch is a generated tile: one small texture plus repeat sampling.
            Paint::Hatch { .. } => self.generated_tiles += 1,
        }
    }

    /// True when a GPU backend could draw this list with the features listed in this
    /// module's documentation.
    pub fn is_renderable(&self) -> bool {
        self.unsupported.is_empty()
    }

    /// A one-line summary for logs and the dev overlay.
    pub fn summary(&self) -> String {
        format!(
            "{} fills, {} strokes, {} segments, {} glyphs in {} runs, {} textures, {} gradients, {} offscreen passes, clips: {} scissor / {} stencil{}",
            self.fill_paths,
            self.stroke_paths,
            self.path_segments,
            self.glyphs,
            self.text_runs,
            self.textures,
            self.gradients,
            self.offscreen_passes,
            self.scissor_clips,
            self.stencil_clips,
            if self.unsupported.is_empty() {
                String::new()
            } else {
                format!(", {} UNSUPPORTED", self.unsupported.len())
            }
        )
    }
}

#[derive(Debug)]
pub enum Error {
    /// The backend is not implemented. Callers fall back to Canvas2D.
    NotImplemented,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the WebGPU backend is not implemented yet; use the canvas2d backend"
        )
    }
}

/// A [`Renderer`] that measures instead of drawing.
///
/// Constructing one is how a host asks "what would the GPU backend need for this slide?"
/// without a GPU. It deliberately implements the same trait: if the trait ever grows a
/// method the GPU cannot satisfy, this stops compiling, which is the whole point.
#[derive(Debug, Default)]
pub struct WebGpuRenderer {
    requirements: Requirements,
    glyph_keys: Vec<(String, char)>,
    textures: Vec<u32>,
}

impl WebGpuRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn requirements(&self) -> &Requirements {
        &self.requirements
    }
}

impl Renderer for WebGpuRenderer {
    type Error = std::convert::Infallible;

    fn begin_frame(&mut self, _dl: &DisplayList, _root: Transform) -> Result<(), Self::Error> {
        self.requirements = Requirements::default();
        self.glyph_keys.clear();
        self.textures.clear();
        Ok(())
    }

    fn execute(&mut self, cmd: &Command) -> Result<(), Self::Error> {
        // Reuse the analyser on a one-command list so there is exactly one definition of
        // what each command costs.
        let mut one = DisplayList::new(0.0, 0.0);
        one.push(cmd.clone());
        let r = Requirements::analyse(&one);
        self.requirements.fill_paths += r.fill_paths;
        self.requirements.stroke_paths += r.stroke_paths;
        self.requirements.path_segments += r.path_segments;
        self.requirements.text_runs += r.text_runs;
        self.requirements.scissor_clips += r.scissor_clips;
        self.requirements.stencil_clips += r.stencil_clips;
        self.requirements.gradients += r.gradients;
        self.requirements.offscreen_passes += r.offscreen_passes;
        self.requirements.generated_tiles += r.generated_tiles;
        self.requirements.unsupported.extend(r.unsupported);
        if let Command::DrawText(run) = cmd {
            let key = run.font.to_css();
            for ch in run.text.chars().filter(|c| !c.is_whitespace()) {
                let pair = (key.clone(), ch);
                if !self.glyph_keys.contains(&pair) {
                    self.glyph_keys.push(pair);
                }
            }
        }
        if let Command::DrawImage { image, .. } = cmd {
            if !self.textures.contains(&image.0) {
                self.textures.push(image.0);
            }
        }
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        self.requirements.glyphs = self.glyph_keys.len();
        self.requirements.textures = self.textures.len();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pptx_core::dl::{
        Color, Command, FontSpec, Gradient, GradientStop, ImageId, Paint, Path, Point, Rect,
        Stroke, TextRun, View,
    };

    fn text(s: &str, family: &str, with_advances: bool) -> Command {
        let n = s.chars().count();
        Command::DrawText(TextRun {
            link: None,
            text: s.into(),
            font: FontSpec::new(family, 12.0),
            origin: Point::new(0.0, 0.0),
            paint: Paint::Solid(Color::BLACK),
            advances: if with_advances {
                vec![6.0; n]
            } else {
                Vec::new()
            },
            width: 6.0 * n as f32,
            decorations: Default::default(),
            letter_spacing: 0.0,
        })
    }

    #[test]
    fn analysis_counts_paths_glyphs_and_textures() {
        let mut dl = DisplayList::new(100.0, 100.0);
        dl.push(Command::FillPath {
            path: Path::rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            paint: Paint::Solid(Color::BLACK),
            rule: Default::default(),
        });
        dl.push(Command::StrokePath {
            path: Path::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)),
            stroke: Stroke::default(),
        });
        dl.push(text("ab", "Arial", true));
        dl.push(Command::DrawImage {
            image: ImageId(3),
            src: Rect::new(0.0, 0.0, 1.0, 1.0),
            dst: Rect::new(0.0, 0.0, 1.0, 1.0),
            opacity: 1.0,
        });
        let r = Requirements::analyse(&dl);
        assert_eq!(r.fill_paths, 1);
        assert_eq!(r.stroke_paths, 1);
        assert_eq!(r.glyphs, 2);
        assert_eq!(r.text_runs, 1);
        assert_eq!(r.textures, 1);
        assert!(r.is_renderable(), "{:?}", r.unsupported);
    }

    #[test]
    fn glyphs_are_deduplicated_per_font_but_not_across_fonts() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(text("aa", "Arial", true));
        dl.push(text("a", "Arial", true));
        dl.push(text("a", "Georgia", true));
        let r = Requirements::analyse(&dl);
        assert_eq!(r.glyphs, 2, "one 'a' per face");
    }

    #[test]
    fn whitespace_needs_no_atlas_space() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(text("a b", "Arial", true));
        assert_eq!(Requirements::analyse(&dl).glyphs, 2);
    }

    #[test]
    fn rect_clips_are_scissors_and_path_clips_need_a_stencil() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::ClipRect(Rect::new(0.0, 0.0, 5.0, 5.0)));
        dl.push(Command::ClipPath {
            path: Path::ellipse(Rect::new(0.0, 0.0, 5.0, 5.0)),
            rule: Default::default(),
        });
        let r = Requirements::analyse(&dl);
        assert_eq!(r.scissor_clips, 1);
        assert_eq!(r.stencil_clips, 1);
    }

    #[test]
    fn gradients_are_counted_so_the_shader_path_is_known_to_be_needed() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(Command::FillPath {
            path: Path::rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            paint: Paint::Gradient(Gradient::Linear {
                start: Point::new(0.0, 0.0),
                end: Point::new(10.0, 0.0),
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: Color::BLACK,
                    },
                    GradientStop {
                        offset: 1.0,
                        color: Color::WHITE,
                    },
                ],
            }),
            rule: Default::default(),
        });
        assert_eq!(Requirements::analyse(&dl).gradients, 1);
    }

    /// The guard this module exists for: text without advances would force the GPU
    /// backend to grow its own shaper, so it is reported rather than silently accepted.
    #[test]
    fn text_without_advances_is_reported_as_unsupported() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(text("hello", "Arial", false));
        let r = Requirements::analyse(&dl);
        assert!(!r.is_renderable());
        assert!(r
            .unsupported
            .first()
            .map(|s| s.contains("advances"))
            .unwrap_or(false));
    }

    #[test]
    fn the_stub_renderer_produces_the_same_analysis_as_the_walker() {
        let mut dl = DisplayList::new(100.0, 100.0);
        dl.push(Command::Save);
        dl.push(Command::FillPath {
            path: Path::rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            paint: Paint::Solid(Color::BLACK),
            rule: Default::default(),
        });
        dl.push(text("hi", "Arial", true));
        dl.push(Command::Restore);

        let mut backend = WebGpuRenderer::new();
        let _ = crate::render(&mut backend, &dl, &View::default());
        assert_eq!(*backend.requirements(), Requirements::analyse(&dl));
    }

    #[test]
    fn the_summary_mentions_the_unsupported_count_when_there_is_one() {
        let mut dl = DisplayList::new(10.0, 10.0);
        dl.push(text("x", "Arial", false));
        assert!(Requirements::analyse(&dl).summary().contains("UNSUPPORTED"));
        let empty = DisplayList::new(10.0, 10.0);
        assert!(!Requirements::analyse(&empty)
            .summary()
            .contains("UNSUPPORTED"));
    }
}
