# AssetDaemon

AssetDaemon is an open-source Unity asset inspection, preview, and export toolchain for Windows, built with Rust (Tauri 2) and Vue 3.

It parses Unity asset bundles and serialized files directly, builds a persistent SQLite index of asset relationships, and provides fast search, preview, and batch export workflows.

## Features

- **Bundle scanning & parsing** — scan directories of Unity asset bundles, parse serialized file metadata, and inspect scene hierarchies
- **Asset map (SQLite index)** — parsing results are persisted in a local SQLite database so relationship queries and repeated loads are nearly instant; the index is built once with a highly parallel pipeline
- **Asset search** — full-text/class-based search across indexed assets with per-class statistics
- **3D preview** — mesh, animator, and GameObject previews rendered with three.js, with automatic diffuse texture resolution
- **Texture support** — PNG, DDS, TGA decoding plus BC texture decompression, with preview and export
- **Export pipeline** — export meshes (GLB), materials, GameObjects, and animators together with their resolved dependencies; batch export with progress logging
- **Game-specific decryptors** — pluggable decryption modules for protected bundles (Girls' Frontline 2: Exilium, Naraka: Bladepoint, The Magic Blade)
- **Task & log system** — every long-running task registers with the task manager, can be terminated from the log panel, and never blocks the UI
- **Update checker** — checks GitHub Releases for new versions from inside the app (no signing keys required)

## Tech Stack

| Layer | Technology |
| --- | --- |
| Desktop shell | Tauri 2 (Rust) |
| Frontend | Vue 3, TypeScript, Vite, Element Plus, vue-i18n, vue-router |
| 3D rendering | three.js |
| Asset parsing | Custom Rust parsers (lz4_flex, lzma-rs, flate2, image, bcdec_rs) |
| Index storage | SQLite via rusqlite (bundled) |

## Development

### Requirements

- [Bun](https://bun.sh/)
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain, MSVC on Windows)
- Windows (current bundle target is NSIS)

### Common commands

```bash
bun install          # install frontend dependencies
bun run tauri dev    # run the app in development mode (hot reload)
bun run tauri build  # produce a release build and NSIS installer
bun run build        # type-check and build the frontend only
bun run test         # run frontend unit tests (vitest)
```

## Repository Layout

```text
.
|- src/                  Vue 3 frontend (pages, components, utils, i18n)
|- src-tauri/            Rust backend (Tauri commands, parsers, exporters, services)
|  |- src/commands/      All #[tauri::command] entry points
|  |- src/unity/         Unity asset parsing (bundles, serialized files, meshes)
|  |- src/decrypt/       Game-specific decryptor modules
|  |- src/exporter/      GLB/texture export with dependency resolution
|  |- src/service/       Asset map / search index services
|  `- src/common/        Shared utilities (bundle IO, texture readers, panic reporting)
`- public/               Static assets
```

## Project Status

AssetDaemon is an active personal research project, now developed in the open. Releases are published on the [GitHub Releases page](https://github.com/StarBobis/AssetDaemon/releases); the app can check for new versions itself via the Releases API.

## Notice

This tool is provided for learning, research, personal asset inspection, data recovery on assets you are legally allowed to inspect, compatibility analysis, and interoperability experiments. You are solely responsible for confirming that every file you open, scan, decrypt, parse, preview, export, or analyze is lawfully obtained and lawfully used by you, and for complying with all applicable laws, copyright rules, platform policies, and end-user license agreements in your jurisdiction.

The project is informed by prior community work in the Unity asset tooling space, including projects released under GPL or MIT licenses such as AssetStudio and AssetRipper, but does not directly copy their source code.

None of this repository, the tool, or the repository owner is affiliated with, sponsored by, or authorized by Unity Technologies or its affiliates.

This project is provided on an "as-is" basis, without warranty of any kind, and is not officially supported by Unity.

## Acknowledgements

Thanks to the developers, researchers, and reverse-engineering communities whose public work helped shape the surrounding ecosystem.

- [AssetStudio](https://github.com/Perfare/AssetStudio)
- [AssetRipper](https://github.com/AssetRipper/AssetRipper)

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=StarBobis/AssetDaemon&type=Date)](https://star-history.com/#StarBobis/AssetDaemon&Date)
