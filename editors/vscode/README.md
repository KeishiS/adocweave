# AdocWeave for Visual Studio Code

[![Open VSX](https://img.shields.io/open-vsx/v/adocweave/adocweave-vscode?label=Open%20VSX)](https://open-vsx.org/extension/adocweave/adocweave-vscode)

A Visual Studio Code extension for [AdocWeave](https://github.com/KeishiS/adocweave), an AsciiDoc processor.
This extension provides the following features:

- Syntax highlighting
- Diagnostics
- Formatting

## Installation

Install `adocweave.adocweave-vscode` from the extension view of an editor that uses Open VSX, such as VSCodium. The official publisher is the `adocweave` namespace.

```sh
codium --install-extension adocweave.adocweave-vscode
```

Visual Studio Code itself reads the Visual Studio Marketplace, where this extension is not published. Install the verified VSIX from [GitHub Releases](https://github.com/KeishiS/adocweave/releases) manually instead. The file published to Open VSX is the same VSIX.

```sh
code --install-extension adocweave-vscode-<version>.vsix --force
```

## Requirements

The extension and Language Server have independent product versions. Compatibility is determined by the `lspApiVersion` reported by `adocweave-lsp --version --json`. The extension pins one tested `managedLspVersion`, downloads it from the matching `adocweave-lsp/vX.Y.Z` GitHub Release, and verifies its product manifest and checksum before starting it. See `docs/user-guide/release-installation.adoc` in the repository for installation, update, rollback, and verification steps.

## License

MIT OR Apache-2.0
