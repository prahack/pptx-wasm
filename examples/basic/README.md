# pptx-wasm — basic example

A minimal React app using only the documented public API. If this ever needs an
undocumented export or a deep import, the package's surface is wrong.

```sh
npm install
cp ../../fixtures/generated/m4-template.pptx public/deck.pptx   # or drop in your own
npm run dev
```

The file picker takes any `.pptx`, so `public/deck.pptx` is only the starting slide.
