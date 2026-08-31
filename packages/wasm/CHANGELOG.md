# Changelog

## [0.54.1] - 2026-08-30

### Breaking changes

- The Worker and direct entry points now use one `analyze({ source, products, resources? })` request and result contract. Results contain only the requested products.
- `projection` is renamed to `document`, whose result type is `DocumentView`. The former `products`, `parse`, and `renderDiagnostics` result fields are removed.
- The former `process` and separate `preprocess` entry points are removed. `AnalyzeRequest` requires a `source` object and a non-empty `products` object; unknown fields, invalid enum values, `null`, and `false` product values are rejected.
- Worker cancellation and all other Worker or direct-entry failures now use `AdocWeaveError`. Cancellation no longer uses `DOMException`, and errors are no longer thrown as JSON strings.
- The public `WASM_PACKAGE_VERSION` export has been removed. Read the installed package version from package metadata when needed; it is not a runtime compatibility signal.
- WebAssembly package releases now use their own `wasm/vX.Y.Z` tags and are published directly to npm instead of being attached to the native GitHub Release.

### Migration

Replace the former top-level fields with the structured request: move `sourceId` to `source.id`, syntax settings to `source`, diagnostic settings to `products.diagnostics`, HTML settings to `products.html`, and resolved inputs to `resources`.

Before:

```javascript
const result = await analyze({
  sourceId: "guide.adoc",
  source,
  products: { html: true, projection: true },
  renderPolicy: { documentMode: "fragment" },
});
```

After:

```javascript
const result = await analyze({
  source: { id: "guide.adoc", text: source },
  products: {
    html: { documentMode: "fragment" },
    document: true,
  },
});
```

Check whether a requested product field exists before reading it, and catch `AdocWeaveError` for cancellation and other failures.

### Maintenance

- Package sources now live under `packages/wasm` together with the package manifest, README, and changelog.
