# AssetDaemon

AssetDaemon is a Unity asset research toolchain project maintained for personal study, testing, version control, and update distribution.

This repository currently focuses on project documentation and GitHub automation around the tool.

## Overview

- Unity asset research oriented workflow
- Documentation site powered by VitePress 2
- GitHub Pages deployment through GitHub Actions
- Repository structure prepared for long-term iteration

## Documentation

- Online docs: https://starbobis.github.io/AssetDaemon/
- Local docs dev server: `bun run docs:dev`
- Production docs build: `bun run docs:build`

## Development

### Requirements

- [Bun](https://bun.sh/)
- Node.js compatible local environment

### Common commands

```bash
bun install
bun run docs:dev
bun run docs:build
bun run docs:preview
```

## Repository Layout

```text
.
|- docs/                  VitePress documentation source
|- docs/.vitepress/       VitePress site configuration
`- .github/workflows/     GitHub Actions workflows
```

## Project Status

AssetDaemon is an active personal research project. The application itself is currently closed-source, while this repository is used to host documentation, project metadata, and supporting automation.

## Notice

This is a closed-source software project built with Rust and Tauri. It is informed by prior community work in the Unity asset tooling space, including projects released under GPL or MIT licenses, such as AssetStudio, but does not directly copy their source code.

If closed-source tooling is not suitable for your use case, please do not use this project.

None of this repository, the tool, or the repository owner is affiliated with, sponsored by, or authorized by Unity Technologies or its affiliates.

This project is provided on an "as-is" basis and is not officially supported by Unity.

## Acknowledgements

Thanks to the developers, researchers, and reverse-engineering communities whose public work helped shape the surrounding ecosystem.

- [AssetStudio](https://github.com/Perfare/AssetStudio)

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=StarBobis/AssetDaemon&type=Date)](https://star-history.com/#StarBobis/AssetDaemon&Date)
