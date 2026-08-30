# AdocWeave for Visual Studio Code

[![Open VSX](https://img.shields.io/open-vsx/v/adocweave/adocweave?label=Open%20VSX)](https://open-vsx.org/extension/adocweave/adocweave)

A Visual Studio Code extension for [AdocWeave](https://github.com/KeishiS/adocweave), an AsciiDoc processor.
This extension provides the following features:

- Syntax highlighting
- Diagnostics
- Formatting

## Installation

Install `adocweave.adocweave` from Visual Studio Marketplace or Open VSX. The official publisher is the `adocweave` namespace.

```sh
code --install-extension adocweave.adocweave
codium --install-extension adocweave.adocweave
```

## Requirements

The extension requires AdocWeave 0.51.0 or later and starts the Language Server as `adocweave lsp`. It first uses the absolute executable path in the machine-level `adocweave.server.path` setting. When that setting is empty, it searches the extension host's `PATH` for `adocweave`. If neither is available, it downloads the latest stable native executable from the project's GitHub Releases.

To select a version yourself, install `adocweave` by following `docs/user-guide/release-installation.adoc` and set `adocweave.server.path` to its absolute path. The extension uses the capabilities announced by the Language Server during standard Language Server Protocol initialization instead of requiring matching product versions.

## License

MIT OR Apache-2.0
