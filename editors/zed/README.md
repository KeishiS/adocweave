<p align="center">
  <img src="icon.png" width="128" height="128" alt="AdocWeave icon">
</p>

# AdocWeave for Zed

AdocWeave adds diagnostics, navigation, formatting, and other Language Server
features to AsciiDoc documents in Zed. It starts the single `adocweave`
executable with the `lsp` subcommand.

Install Zed's `AsciiDoc` extension first. That extension provides the AsciiDoc
language and syntax highlighting; AdocWeave attaches its Language Server to the
existing `AsciiDoc` language instead of duplicating those files.

The extension looks for the Language Server in this order:

1. The absolute path in `lsp.adocweave.binary.path`.
2. `adocweave` on the environment inherited by Zed.
3. The latest stable native release from the AdocWeave GitHub repository.

Automatic download supports Linux x86_64 and ARM64, macOS ARM64, and Windows
x86_64. AdocWeave 0.51.0 or newer is required. The extension and the native
executable have independent versions and do not need to match.

To use a specific executable, add its absolute path to Zed settings:

```json
{
  "lsp": {
    "adocweave": {
      "binary": {
        "path": "/absolute/path/to/adocweave"
      }
    }
  }
}
```

Install `AsciiDoc` from the Zed Extensions view. If `AdocWeave` is also listed
there, install it from that view. To use a repository checkout directly, choose
**Install Dev Extension** and select this `editors/zed` directory.

The extension does not bundle the Language Server. Zed's extension API does not
expose downloaded bytes for checksum verification, so automatic download relies
on HTTPS. Install and verify the native archive yourself when stronger integrity
checks are required.

## License

The extension is available under the [MIT License](LICENSE).
