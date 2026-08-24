export const MAX_WASM_ARCHIVE_BYTES = 2 * 1024 * 1024;
export const MAX_WASM_MODULE_BYTES = 1280 * 1024;

export function wasmArtifactSizeError(archiveBytes, wasmBytes) {
  if (archiveBytes > MAX_WASM_ARCHIVE_BYTES) {
    return `archive exceeds 2 MiB: ${archiveBytes}`;
  }
  if (wasmBytes > MAX_WASM_MODULE_BYTES) {
    return `WASM exceeds 1.25 MiB: ${wasmBytes}`;
  }
  return null;
}

export function assertWasmArtifactSizes(archiveBytes, wasmBytes) {
  const error = wasmArtifactSizeError(archiveBytes, wasmBytes);
  if (error !== null) throw new Error(error);
}
