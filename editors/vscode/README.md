# AdocWeave for Visual Studio Code

[![Open VSX](https://img.shields.io/open-vsx/v/adocweave/adocweave-vscode?label=Open%20VSX)](https://open-vsx.org/extension/adocweave/adocweave-vscode)

A Visual Studio Code extension for [AdocWeave](https://github.com/KeishiS/adocweave), an AsciiDoc processor.
This extension provides the following features:

- Syntax highlighting
- Diagnostics
- Formatting

## Installation

Install `adocweave.adocweave-vscode` from Visual Studio Marketplace or Open VSX. The official publisher is the `adocweave` namespace.

```sh
code --install-extension adocweave.adocweave-vscode
codium --install-extension adocweave.adocweave-vscode
```

You can also install the verified VSIX from [GitHub Releases](https://github.com/KeishiS/adocweave/releases). The same extension version is published to both registries and attached to the corresponding `vX.Y.Z` release.

```sh
code --install-extension adocweave-vscode-<version>.vsix --force
```

## Requirements

The extension starts the Language Server as `adocweave lsp`. It first uses the absolute executable path in the machine-level `adocweave.server.path` setting. When that setting is empty, it searches the extension host's `PATH` for `adocweave`. If neither is available, it downloads the latest stable native executable from the project's GitHub Releases.

To select a version yourself, install `adocweave` by following `docs/user-guide/release-installation.adoc` and set `adocweave.server.path` to its absolute path. The extension and Language Server negotiate supported features through the standard Language Server Protocol initialization.

## License

MIT OR Apache-2.0
