<p align="center">
  <img src="resources/icon.png" width="128" height="128" alt="AdocWeave icon">
</p>

# AdocWeave for Visual Studio Code

[![Visual Studio Marketplace](https://img.shields.io/badge/VS%20Marketplace-install-007ACC?logo=visualstudiocode&logoColor=white)](https://marketplace.visualstudio.com/items?itemName=adocweave.adocweave)
[![Open VSX](https://img.shields.io/open-vsx/v/adocweave/adocweave?label=Open%20VSX)](https://open-vsx.org/extension/adocweave/adocweave)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE)

AdocWeave adds AsciiDoc syntax highlighting, diagnostics, navigation, completion, and formatting to Visual Studio Code and compatible editors.

## Installation

Install `adocweave.adocweave` from the [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=adocweave.adocweave) or [Open VSX](https://open-vsx.org/extension/adocweave/adocweave). The official publisher is `adocweave`.

```sh
code --install-extension adocweave.adocweave
codium --install-extension adocweave.adocweave
```

## Language Server

The extension requires AdocWeave 0.51.0 or later. It uses the first available executable from:

1. The absolute path in the machine-level `adocweave.server.path` setting.
2. `adocweave` on the extension host's `PATH`.
3. The latest stable AdocWeave release, downloaded automatically.

The extension and executable versions do not need to match. After changing `adocweave.server.path`, run **AdocWeave: Restart Language Server**.

See [Installation and updates](https://github.com/KeishiS/adocweave/blob/main/docs/user-guide/release-installation.adoc#vscode-installation) to install or verify the executable yourself.

## Development

See [VS Code extension development](https://github.com/KeishiS/adocweave/blob/main/docs/developer-guide/vscode-development.adoc) to build and test a repository checkout.

## License

Licensed under [MIT OR Apache-2.0](LICENSE).
