# AdocWeave for Visual Studio Code

A Visual Studio Code extension for [AdocWeave](https://github.com/KeishiS/adocweave), an AsciiDoc processor.
This extension provides the following features:

- Syntax highlighting
- Diagnostics
- Formatting

## Installation

After `adocweave.adocweave` is listed in Visual Studio Marketplace or Open VSX, install it with the command for your editor. The official publisher is the `adocweave` namespace.

```sh
code --install-extension adocweave.adocweave
codium --install-extension adocweave.adocweave
```

Until it is listed, build and install a VSIX from a repository checkout:

```sh
nix develop
npm ci --ignore-scripts --prefix editors/vscode
npm run package --prefix editors/vscode
VERSION="$(node -p "require('./editors/vscode/package.json').version")"
code --install-extension "target/distrib/adocweave-$VERSION.vsix"
```

## Requirements

The extension requires AdocWeave 0.51.0 or later and starts the Language Server as `adocweave lsp`. It first uses the absolute executable path in the machine-level `adocweave.server.path` setting. When that setting is empty, it searches the extension host's `PATH` for `adocweave`. If neither is available, it downloads the latest stable native executable from the project's GitHub Releases.

To select a version yourself, install `adocweave` by following `docs/user-guide/release-installation.adoc` and set `adocweave.server.path` to its absolute path. The extension uses the capabilities announced by the Language Server during standard Language Server Protocol initialization instead of requiring matching product versions.

## License

MIT OR Apache-2.0
