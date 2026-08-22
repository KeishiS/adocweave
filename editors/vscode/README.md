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

Install `adocweave-lsp` separately before using the language features. The extension first uses the absolute executable path in the machine-level `adocweave.server.path` setting. When that setting is empty, it searches the extension host's `PATH`.

The extension does not download or update the Language Server. If it cannot find the executable, follow the Language Server installation instructions in `docs/user-guide/release-installation.adoc` in the repository. The extension and Language Server negotiate supported features through the standard Language Server Protocol initialization.

## License

MIT OR Apache-2.0
