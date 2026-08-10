# StorageSlim

StorageSlim is a Tauri desktop app for local-first image optimization.

## Current MVP

- Multiple file selection
- Folder input with recursive traversal
- Drag and drop
- Resize by width, height, or long edge
- Format conversion for GIF / JPEG / PNG / WebP / AVIF
- Animated GIF protection rules
- Output path control with folder structure preservation
- Timestamp carry-over
- Batch result reporting with Saved size aggregation

## Notes

- HEIC / HEIF input and original-format re-export remain build-dependent and are not enabled in the current MVP runtime.
- Metadata preservation is exposed as a setting, but the current MVP build treats it as unsupported and returns warnings.

## Local Commands

- `npm run build`
- `npm run tauri build -- --debug --no-bundle`
- `cargo test --manifest-path src-tauri/Cargo.toml`
