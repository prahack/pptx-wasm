# Changelog

Notable changes to `pptx-wasm`. Dates are release dates; the payload figure is the gzipped
module plus its JS glue, which is what a browser downloads.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project uses [semantic versioning](https://semver.org/) — pre-1.0, so the minor version
carries breaking changes.

## 0.3.0

**Added**

- **Search with bounds.** `deck.searchSlide(index, query, options)` returns each match with
  the rectangles to highlight it; `deck.findSlides(query)` reports which slides contain it
  and how often. Matches are positioned by the same walk that painted the glyphs, so a
  highlight cannot land where the text is not. A match spanning several runs returns one
  rectangle per run rather than one box across the gaps, and a phrase is never matched
  across a line break.
- **Hyperlinks.** Every run on the text layer now carries the URL it links to, or `null`.
  `hlinkClick` was already being parsed and then dropped; nothing downstream ever saw it.
  Only `http`, `https` and `mailto` survive — a deck is untrusted input and the URL is
  about to be put in an `href`, so schemes are allow-listed rather than sanitised.

**Changed**

- Payload is 332 KB, up from 328. The benchmark table in the README was measured at 0.2.0
  and is a release behind.

## 0.2.0

**Added**

- **34 preset geometries**: 22 `flowChart*` shapes and all 12 `actionButton*` shapes. Found
  by diffing capabilities against another renderer — 22 of them had been falling back to a
  plain rectangle, and no fixture covered them, so nothing said so.
- **Soft edges** (`<a:softEdge>`), as a display-list group whose alpha is feathered inward
  from the silhouette.
- **Selectable text.** `<PresentationViewer selectableText />` lays a transparent span per
  run over the canvas. Screen readers and find-in-page never needed it — the component has
  always rendered an off-screen copy of the slide text — but selection is a pointer
  interaction an off-screen block cannot serve.
- **`deck.textLayer(index, options)`**, the data behind that overlay: per-run text,
  baseline, measured width, size, family, weight, slant and rotation, in device pixels.
- **Cargo features `charts` and `tables`**, both on by default. Measured at the time:
  319.6 KB with both, 272.9 KB with neither. A deck that uses a disabled feature still
  parses and renders everything else on the slide; only that shape's frame is empty.

**Fixed**

- Five preset curves that had been wrong since M1 and were never covered by a fixture:
  `terminator` was a stadium where the spec gives elliptical caps at `0.161w`;
  `inputOutput` scaled its slant by the shorter side where the spec says the width;
  `document`, `punchedTape` and `multidocument` had the wrong wave.
- `Path::bounds()` measured the hull of a curve's control points, which for a quarter
  circle sits about 5% outside the arc. It now solves the real extrema, which stopped it
  rejecting correct geometry and lets the renderer cull more.
- Every font fallback chain now ends in a concrete face before its generic. A generic
  family is resolved by the browser, and Chromium, Firefox and WebKit do not resolve it
  the same way — a deck could wrap differently in each. Found by CI on Linux, invisible on
  macOS where all three go through Core Text.

## 0.1.1

**Changed**

- Documentation only. The npm page only refreshes on publish, and 0.1.0 shipped with the
  pre-benchmark README.

## 0.1.0

First release. Rust/WASM core, TypeScript API, optional React component: OOXML parsing,
the full inheritance chain, text layout, ~70 preset geometries plus custom geometry,
tables, charts, images, gradients, hatch and tiled fills, shadows and glow.
