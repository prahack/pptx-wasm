# pptx-viewer

Render PowerPoint `.pptx` files in the browser. No server, no conversion step, no upload —
the file is parsed and drawn on the client.

The parser, layout engine and renderer are Rust compiled to WebAssembly. This package is
the TypeScript API around it, plus an optional React component.

```tsx
import { PresentationViewer } from 'pptx-viewer/react';

<PresentationViewer src="/deck.pptx" width="100%" height="100vh" />
```

## Install

```sh
npm install pptx-viewer
```

React is an optional peer dependency; you only need it for `pptx-viewer/react`.

## What it renders

| | |
|---|---|
| **Text** | paragraphs, runs, alignment, word wrap, line and paragraph spacing, bold/italic/underline/strikethrough, bullets and auto-numbering, vertical anchoring, autofit, embedded fonts |
| **Shapes** | ~70 preset geometries, custom geometry (including the DrawingML formula language), connectors, nested groups with rotation and flips |
| **Fills** | solid, linear and radial gradients, picture fills, pattern approximations, transparency |
| **Images** | PNG, JPEG, GIF, BMP, WebP, SVG, with `srcRect` cropping and shape-clipping |
| **Inheritance** | the full chain: shape → placeholder → layout → master → theme, with colour maps and theme fonts |
| **Tables** | grids, merged cells, borders, and PowerPoint's built-in table styles |
| **Charts** | bar, column, line, area, pie, doughnut and scatter, with axes, gridlines and legends |
| **Effects** | drop shadows and glow |

Known gaps: soft edges, 3-D bevels, SmartArt and OLE embeddings (these render as their
fallback image where the file provides one), animations and transitions, and EMF/WMF
images — which no browser can decode.

## Framework-agnostic API

```ts
import { Presentation } from 'pptx-viewer';

const deck = await Presentation.open('/deck.pptx');
const canvas = document.querySelector('canvas');

await deck.render(0, canvas, { fit: 'contain' });

console.log(deck.slideCount, deck.slideSize);
deck.destroy();
```

### `Presentation.open(src, options?)`

`src` is a URL, `File`, `Blob`, `ArrayBuffer` or `Uint8Array`.

| option | default | |
|---|---|---|
| `wasm` | bundled | URL, `Response` or `ArrayBuffer` for the `.wasm` module |
| `useEmbeddedFonts` | `true` | install fonts embedded in the deck as `FontFace`s |
| `signal` | — | `AbortSignal`, honoured while fetching a URL |

### `deck.render(index, canvas, options?)`

Draws a slide. Sizes the canvas backing store from `options` and the device pixel ratio.

| option | default | |
|---|---|---|
| `width`, `height` | the canvas's client size | in CSS pixels |
| `dpr` | `devicePixelRatio` | |
| `fit` | `'contain'` | `'contain'`, `'cover'`, `'fill'`, `'actual'` |
| `zoom` | `1` | applied on top of the fit |
| `panX`, `panY` | `0` | in CSS pixels |

Resolves to `{ complete }`. `complete: false` means images were still decoding — subscribe
to `onAssetsReady` and draw again.

### Other members

| | |
|---|---|
| `deck.info` | `{ slideCount, width, height, slides, embeddedFonts }`; size in points |
| `deck.slideCount`, `deck.slideSize` | |
| `deck.prepare(index)` | lay a slide out without drawing it — worth doing for neighbours |
| `deck.text(index)` | the slide's text, in draw order |
| `deck.notes(index)` | speaker notes |
| `deck.onAssetsReady(fn)` | called when late-decoding images arrive; returns an unsubscribe |
| `deck.pendingAssetCount()` | images still decoding |
| `deck.evict()` | drop cached layouts under memory pressure |
| `deck.destroy()` | free the WASM-side deck and uninstall embedded fonts |

`deck.debugTrace(index)` and `deck.gpuRequirements(index)` are diagnostics. Their output
format is not stable; do not build on them.

## React

```tsx
import { useRef } from 'react';
import { PresentationViewer, type PresentationViewerHandle } from 'pptx-viewer/react';

function Deck() {
  const viewer = useRef<PresentationViewerHandle>(null);
  return (
    <>
      <PresentationViewer
        ref={viewer}
        src="/deck.pptx"
        height="70vh"
        onLoad={(info) => console.log(`${info.slideCount} slides`)}
      />
      <button onClick={() => viewer.current?.next()}>Next</button>
    </>
  );
}
```

### Props

| prop | default | |
|---|---|---|
| `src` | — | URL, `File`, `Blob`, `ArrayBuffer` or `Uint8Array` |
| `width`, `height` | `100%` | CSS length or number of pixels |
| `initialSlide` | `0` | |
| `slide` | — | pass to control the index yourself |
| `fit` | `'contain'` | |
| `zoom` | `1` | |
| `keyboard` | `true` | arrows, Page Up/Down, Home/End, space |
| `wasm` | bundled | where to load the WASM module from |
| `loading` | a spinner | node shown while the deck loads |
| `renderError` | a message | `(error) => ReactNode` |
| `className`, `style` | — | |
| `onLoad` | — | `(info: PresentationInfo) => void` |
| `onError` | — | `(error: PptxError) => void` |
| `onSlideChange` | — | `(index: number) => void` |

The component redraws on resize and on device-pixel-ratio changes, prefetches the
neighbouring slides, and exposes the current slide's text to assistive technology and
find-in-page.

### Handle

`next()`, `previous()`, `goTo(index)`, `redraw()`, and the read-only `slide`,
`slideCount` and `presentation`.

## Loading the WASM module

The package ships `pptx_bg.wasm` as a real file next to the JS rather than inlining it, so
it is cached separately and compiled while it streams. Most bundlers resolve it with no
configuration.

To serve it from elsewhere — a CDN, or a path your framework controls:

```ts
import { initWasm } from 'pptx-viewer';
await initWasm('https://cdn.example.com/pptx_bg.wasm');
```

Call it once before the first `Presentation.open`, or pass the same value as the `wasm`
prop. The module is instantiated once per page and shared.

## Browser support

Chrome, Edge, Firefox and Safari — verified per release by rendering the whole fixture
suite in Chromium, Firefox and WebKit and checking the output matches.

Requires WebAssembly, `OffscreenCanvas` and `Path2D`, all available since 2020.

## A note on fidelity

Text is measured with the browser's own `measureText`, so line breaks and drawn glyphs
come from one engine and cannot disagree. The cost is that the three browser engines do
not return byte-identical advances, so a paragraph could in principle break differently
between them. Advances are measured in points at the authored size, never at the zoomed
size, so a deck breaks identically at every zoom level and DPR — and the font fallback
chain prefers metric-compatible substitutes (Carlito for Calibri, Liberation Sans for
Arial) so a missing face keeps its wrap points instead of reflowing the slide.

## Licence

MIT
