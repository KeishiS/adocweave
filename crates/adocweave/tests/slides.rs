use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn convert(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_adocweave"))
        .current_dir(root)
        .arg("convert")
        .args(arguments)
        .output()
        .expect("adocweave command")
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(root: &Path, path: &str, content: &str) {
    fs::write(root.join(path), content).unwrap();
}

const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><rect width=\"80\" height=\"80\" fill=\"#123\"/><text x=\"2\" y=\"20\">結果</text></svg>";

#[test]
fn slide_aspect_ratio_defaults_to_auto_and_uses_only_header_attributes() {
    for header in [
        "",
        ":slides-aspect-ratio: auto\n",
        ":slides-aspect-ratio: 16:9\n:slides-aspect-ratio!:\n",
    ] {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            "talk.adoc",
            &format!("= Talk\n{header}\n:slides-aspect-ratio: invalid\n\n== Slide\n\nBody.\n"),
        );
        success(&convert(
            root.path(),
            &[
                "--no-config",
                "talk.adoc",
                "--to",
                "revealjs",
                "--output",
                "dist",
            ],
        ));
        let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
        assert!(!html.contains("data-aspect-ratio="), "{html}");
    }
}

#[test]
fn slide_aspect_ratio_accepts_fixed_and_referenced_header_values() {
    for (header, expected) in [
        (":slides-aspect-ratio: 16:9\n", "16:9"),
        (":slides-aspect-ratio: 16:10\n", "8:5"),
        (":slides-aspect-ratio: 4:3\n", "4:3"),
        (":ratio: 1920:1080\n:slides-aspect-ratio: {ratio}\n", "16:9"),
        (":slides-aspect-ratio: 1:4\n", "1:4"),
        (":slides-aspect-ratio: 4:1\n", "4:1"),
    ] {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            "talk.adoc",
            &format!("= Talk\n{header}\n:slides-aspect-ratio: invalid\n\n== Slide\n\nBody.\n"),
        );
        success(&convert(
            root.path(),
            &[
                "--no-config",
                "talk.adoc",
                "--to",
                "revealjs",
                "--output",
                "dist",
            ],
        ));
        let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
        assert!(
            html.contains(&format!("data-aspect-ratio=\"{expected}\"")),
            "{html}"
        );
    }
}

#[test]
fn configured_slide_aspect_ratio_overrides_header_values_and_unsetting() {
    for (configuration, header, expected) in [
        (
            "value = \"4:3\"",
            ":slides-aspect-ratio: 16:9\n",
            Some("4:3"),
        ),
        ("value = \"16:10\"", ":slides-aspect-ratio!:\n", Some("8:5")),
        ("value = \"auto\"", ":slides-aspect-ratio: 16:9\n", None),
        ("unset = true", ":slides-aspect-ratio: 16:9\n", None),
    ] {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            ".adocweave.toml",
            &format!(
                "schema-version = 2\n[analysis.attributes.slides-aspect-ratio]\n{configuration}\n"
            ),
        );
        write(
            root.path(),
            "talk.adoc",
            &format!("= Talk\n{header}\n== Slide\n\nBody.\n"),
        );
        success(&convert(
            root.path(),
            &["talk.adoc", "--to", "revealjs", "--output", "dist"],
        ));
        let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
        if let Some(ratio) = expected {
            assert!(
                html.contains(&format!("data-aspect-ratio=\"{ratio}\"")),
                "{html}"
            );
        } else {
            assert!(!html.contains("data-aspect-ratio="), "{html}");
        }
    }
}

#[test]
fn invalid_slide_aspect_ratio_has_original_source_and_preserves_the_output_directory() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\nOriginal.\n",
    );
    success(&convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
        ],
    ));
    let files = ["index.html", ".adocweave-manifest.json"]
        .map(|file| (file, fs::read(root.path().join("dist").join(file)).unwrap()));
    for value in [
        "",
        "wide",
        "0:9",
        "16:0",
        "1:5",
        "5:1",
        "1.5:1",
        "16:9:1",
        "4294967296:1",
        "\" onload=\"alert(1)",
    ] {
        write(
            root.path(),
            "talk.adoc",
            &format!("= Talk\n:slides-aspect-ratio: {value}\n\n== Slide\n\nBody.\n"),
        );
        let output = convert(
            root.path(),
            &[
                "--no-config",
                "talk.adoc",
                "--to",
                "revealjs",
                "--output",
                "dist",
            ],
        );
        assert!(!output.status.success(), "{value}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("error[slides-invalid-aspect-ratio]"),
            "{stderr}"
        );
        assert!(stderr.contains("talk.adoc:2:"), "{stderr}");
        for (file, original) in &files {
            assert_eq!(
                fs::read(root.path().join("dist").join(file)).unwrap(),
                *original
            );
        }
    }
    write(
        root.path(),
        "talk.adoc",
        "= Talk\ninclude::settings.adoc[]\n\n== Slide\n\nBody.\n",
    );
    write(
        root.path(),
        "settings.adoc",
        ":slides-aspect-ratio: invalid\n",
    );
    let output = convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("settings.adoc:1:"), "{stderr}");
    assert!(
        stderr.contains("error[slides-invalid-aspect-ratio]"),
        "{stderr}"
    );
    for (file, original) in &files {
        assert_eq!(
            fs::read(root.path().join("dist").join(file)).unwrap(),
            *original
        );
    }
}

#[test]
fn invalid_configured_slide_aspect_ratio_is_a_usage_error() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        ".adocweave.toml",
        "schema-version = 2\n[analysis.attributes.slides-aspect-ratio]\nvalue = \"invalid\"\n",
    );
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n:slides-aspect-ratio: 16:9\n\n== Slide\n\nBody.\n",
    );
    let output = convert(
        root.path(),
        &["talk.adoc", "--to", "revealjs", "--output", "dist"],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("slides-aspect-ratio"), "{stderr}");
    assert!(!stderr.contains("talk.adoc:"), "{stderr}");
    assert!(
        !stderr.contains("error[slides-invalid-aspect-ratio]"),
        "{stderr}"
    );
    assert!(!root.path().join("dist").exists());
}

#[cfg(unix)]
#[test]
fn helper_execution_does_not_inherit_project_node_loader_or_relative_path_entries() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let installation = tempfile::tempdir().unwrap();
    let node = Command::new("node")
        .args(["-p", "process.execPath"])
        .output()
        .unwrap();
    assert!(node.status.success());
    let node = std::path::PathBuf::from(String::from_utf8(node.stdout).unwrap().trim());
    let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/slides-helper/bin.mjs")
        .canonicalize()
        .unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Math\n\nlatexmath:[x^2]\n",
    );
    write(
        root.path(),
        "node",
        "#!/bin/sh\nprintf project-node > \"$TEST_NODE_MARKER\"\nexit 9\n",
    );
    fs::set_permissions(root.path().join("node"), fs::Permissions::from_mode(0o700)).unwrap();
    write(
        root.path(),
        "preload.cjs",
        "require('node:fs').writeFileSync(process.env.TEST_PRELOAD_MARKER, 'preload');\n",
    );
    symlink(&helper, installation.path().join("adocweave-slides-helper")).unwrap();
    write(
        installation.path(),
        "trusted-helper",
        "#!/bin/sh\nprintf '%s' \"$PWD\" > \"$TEST_CWD_MARKER\"\nexec \"$TEST_REAL_NODE\" \"$TEST_HELPER_MODULE\"\n",
    );
    fs::set_permissions(
        installation.path().join("trusted-helper"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let path = std::env::join_paths([Path::new("."), installation.path(), node.parent().unwrap()])
        .unwrap();
    for (output_directory, explicit) in [("default", false), ("explicit", true)] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_adocweave"));
        command
            .current_dir(root.path())
            .args([
                "convert",
                "talk.adoc",
                "--to",
                "revealjs",
                "--output",
                output_directory,
            ])
            .env("PATH", &path)
            .env("NODE_OPTIONS", "--require ./preload.cjs")
            .env("NODE_PATH", ".")
            .env("TEST_NODE_MARKER", root.path().join("node-started"))
            .env("TEST_PRELOAD_MARKER", root.path().join("preload-started"))
            .env("TEST_CWD_MARKER", root.path().join("helper-cwd"))
            .env("TEST_REAL_NODE", &node)
            .env("TEST_HELPER_MODULE", &helper);
        if explicit {
            command
                .arg("--slides-helper")
                .arg(installation.path().join("trusted-helper"));
        } else {
            command.env_remove("ADOCWEAVE_SLIDES_HELPER");
        }
        success(&command.output().unwrap());
        assert!(
            fs::read_to_string(root.path().join(output_directory).join("index.html"))
                .unwrap()
                .contains("math-rendered")
        );
    }
    assert!(!root.path().join("node-started").exists());
    assert!(!root.path().join("preload-started").exists());
    assert_eq!(
        fs::read_to_string(root.path().join("helper-cwd")).unwrap(),
        installation
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
}

#[test]
fn public_is_offline_and_private_resources_are_not_even_acquired() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Research\n\n[#method]\n== Method\n\n[%step]\n* One\n* Two\n\nimage::first.svg[Result]\n\n[.notes]\n--\nPRIVATE_NOTE\nimage::private.svg[PRIVATE_IMAGE]\nstem:[x^2]\ncite:[private]\n--\n\n=== Vertical\n\n== Last\n\n<<method>>\n",
    );
    write(root.path(), "first.svg", SVG);
    // There is deliberately no private.svg and no helper executable.
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist/talk",
            "--slides-helper",
            "/missing/helper",
            "--bibliography",
            "missing.json",
            "--csl-style",
            "missing-style.xml",
            "--csl-locale",
            "missing-locale.xml",
            "--math-macros",
            "missing-macros.json",
        ],
    );
    success(&output);
    assert!(output.stdout.is_empty());
    let html = fs::read_to_string(root.path().join("dist/talk/index.html")).unwrap();
    assert!(html.contains("data-fragment-index=\"0\""));
    assert!(html.contains("data-fragment-index=\"1\""));
    assert!(html.contains("id=\"method\""));
    assert!(html.contains("href=\"#method\""));
    assert!(html.contains("<img src=\"assets/"));
    assert!(!html.contains("PRIVATE"));
    assert!(!html.contains("private.svg"));
    assert!(!html.contains("notes.js"));
    let manifest =
        fs::read_to_string(root.path().join("dist/talk/.adocweave-manifest.json")).unwrap();
    assert!(!manifest.contains("private"));
    assert!(!manifest.contains("first.svg"));
    assert!(manifest.contains("sizeBytes"));
    let reader = adocweave_project::open_managed_bundle(
        &root.path().join("dist/talk").canonicalize().unwrap(),
        Default::default(),
        &adocweave_core::NeverCancel,
    )
    .unwrap();
    let image = reader
        .manifest()
        .files
        .iter()
        .find(|file| file.media_type == adocweave_project::BundleMediaType::Svg)
        .unwrap();
    assert!(
        String::from_utf8(reader.read_file(&image.path).unwrap().1)
            .unwrap()
            .contains("結果")
    );
}

#[test]
fn presenter_to_public_removes_private_images_plugin_and_manifest_entries() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\nPublic.\n\n[.notes]\n--\nPRIVATE_NOTE\nimage::private.svg[PRIVATE_ALT]\n--\n",
    );
    write(root.path(), "private.svg", SVG);
    success(&convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--audience",
            "presenter",
        ],
    ));
    let old = adocweave_project::open_managed_bundle(
        &root.path().join("dist").canonicalize().unwrap(),
        Default::default(),
        &adocweave_core::NeverCancel,
    )
    .unwrap();
    let private_image = old
        .manifest()
        .files
        .iter()
        .find(|file| file.media_type == adocweave_project::BundleMediaType::Svg)
        .unwrap()
        .path
        .clone();
    assert!(
        fs::read_to_string(root.path().join("dist/index.html"))
            .unwrap()
            .contains("PRIVATE_NOTE")
    );
    assert!(
        fs::read_to_string(root.path().join("dist/index.html"))
            .unwrap()
            .contains("<img src=\"assets/")
    );
    success(&convert(
        root.path(),
        &["talk.adoc", "--to", "revealjs", "--output", "dist"],
    ));
    assert!(!root.path().join("dist").join(private_image).exists());
    assert!(!root.path().join("dist/assets/notes.js").exists());
    assert!(!root.path().join("dist/licenses/marked.txt").exists());
    for file in ["index.html", ".adocweave-manifest.json"] {
        assert!(
            !fs::read_to_string(root.path().join("dist").join(file))
                .unwrap()
                .contains("PRIVATE")
        );
    }
}

#[test]
fn ordinary_convert_still_keeps_notes_and_writes_html_to_stdout() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n[.notes]\n--\nPRIVATE_NOTE\n--\n",
    );
    let output = convert(root.path(), &["talk.adoc"]);
    success(&output);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("PRIVATE_NOTE")
    );
    assert!(!root.path().join(".adocweave-manifest.json").exists());
}

#[test]
fn helper_failures_and_invalid_local_data_never_save_raw_math() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n:stem: latexmath\n\n== Slide\n\nlatexmath:[E=mc^2].\n",
    );
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--slides-helper",
            "/missing/helper",
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("slides-helper-not-found"));
    assert!(!root.path().join("dist").exists());
    write(root.path(), "macros.json", "[{\n invalid }]");
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--math-macros",
            "macros.json",
            "--slides-helper",
            "/missing/helper",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("macros.json:2:"), "{stderr}");
    assert!(stderr.contains("invalid math macro JSON"), "{stderr}");
    assert!(!stderr.contains("slides-helper-not-found"), "{stderr}");
    assert!(!root.path().join("dist").exists());
}

#[test]
fn visible_citations_require_explicit_data_and_reject_manual_key_collisions_first() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\ncite:[shared].\n",
    );
    let output = convert(
        root.path(),
        &["talk.adoc", "--to", "revealjs", "--output", "dist"],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("supply --bibliography, --csl-style, and --csl-locale")
    );
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\ncite:[shared].\n\n[bibliography]\n* [[[shared]]] Hand-written entry.\n",
    );
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--bibliography",
            "missing.json",
            "--csl-style",
            "missing.xml",
            "--csl-locale",
            "missing-locale.xml",
            "--slides-helper",
            "/missing/helper",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("slides-bibliography-key-conflict"),
        "{stderr}"
    );
    assert!(!stderr.contains("cannot open"), "{stderr}");
    assert!(!root.path().join("dist").exists());
}

#[test]
fn local_css_is_linked_after_the_fixed_theme_in_author_order_and_updated_by_digest() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "talk.adoc", "= Talk\n\n== Slide\n\nText.\n");
    write(root.path(), "z-first.css", ".reveal { color: red; }\n");
    write(root.path(), "a-last.css", ".reveal { color: green; }\n");
    let arguments = [
        "talk.adoc",
        "--to",
        "revealjs",
        "--output",
        "dist",
        "--css",
        "z-first.css",
        "--css",
        "a-last.css",
    ];
    success(&convert(root.path(), &arguments));
    let reader = adocweave_project::open_managed_bundle(
        &root.path().join("dist").canonicalize().unwrap(),
        Default::default(),
        &adocweave_core::NeverCancel,
    )
    .unwrap();
    let find = |content: &[u8]| {
        reader
            .manifest()
            .files
            .iter()
            .find(|file| {
                file.media_type == adocweave_project::BundleMediaType::Css
                    && reader.read_file(&file.path).unwrap().1 == content
            })
            .unwrap()
            .path
            .clone()
    };
    let first = find(b".reveal { color: red; }\n");
    let last = find(b".reveal { color: green; }\n");
    let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
    assert!(html.find("assets/theme.css").unwrap() < html.find(&first).unwrap());
    assert!(html.find(&first).unwrap() < html.find(&last).unwrap());
    write(root.path(), "a-last.css", ".reveal { color: blue; }\n");
    success(&convert(root.path(), &arguments));
    assert!(root.path().join("dist").join(first).exists());
    assert!(!root.path().join("dist").join(last).exists());
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--css-url",
            "https://example.com/theme.css",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--css-url apply only to ordinary HTML")
    );
}

#[test]
fn slide_footnotes_are_per_slide_and_notes_do_not_change_body_numbers() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== First\n\nPublic footnote:shared[Public footnote text].\n\n[.notes]\n--\nPRIVATE_NOTE footnote:private[PRIVATE_FOOTNOTE].\nShared footnote:shared[].\n--\n\n== Last\n\nAgain footnote:shared[].\n",
    );
    let mut body = None;
    for audience in ["public", "presenter"] {
        success(&convert(
            root.path(),
            &[
                "talk.adoc",
                "--to",
                "revealjs",
                "--output",
                audience,
                "--audience",
                audience,
            ],
        ));
        let html = fs::read_to_string(root.path().join(audience).join("index.html")).unwrap();
        assert!(html.contains("slides-body-s2-footnote-1"), "{html}");
        assert!(html.contains("slides-body-s3-footnote-1"), "{html}");
        assert!(!html.contains("slides-body-s2-footnote-2"));
        if audience == "public" {
            assert!(!html.contains("PRIVATE"));
            body = Some(html);
        } else {
            assert!(html.contains("slides-notes-s2-footnote-1"), "{html}");
            assert!(html.contains("slides-notes-s2-footnote-2"), "{html}");
            assert!(html.contains("PRIVATE_FOOTNOTE"));
            assert!(body.as_ref().unwrap().contains("Public footnote text"));
        }
    }
}

#[test]
fn slide_page_keeps_header_language_and_uses_the_ordinary_empty_default() {
    let root = tempfile::tempdir().unwrap();
    for (header, expected) in [(":lang: ja\n", "ja"), ("", "")] {
        write(
            root.path(),
            "talk.adoc",
            &format!("= Talk\n{header}\n== Slide\n\n研究.\n"),
        );
        success(&convert(
            root.path(),
            &["talk.adoc", "--to", "revealjs", "--output", "dist"],
        ));
        assert!(
            fs::read_to_string(root.path().join("dist/index.html"))
                .unwrap()
                .contains(&format!("<html lang=\"{expected}\">"))
        );
    }
}

#[test]
fn output_requires_a_dedicated_directory_and_ordinary_options_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "talk.adoc", "= Talk\n\n== Slide\n\nText.\n");
    for args in [
        vec!["talk.adoc", "--to", "revealjs"],
        vec!["talk.adoc", "--to", "revealjs", "--output", "."],
        vec![
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--complete",
        ],
        vec![
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--css",
            "missing.css",
        ],
        vec!["talk.adoc", "--output", "dist"],
        vec!["talk.adoc", "--audience", "presenter"],
    ] {
        assert!(!convert(root.path(), &args).status.success(), "{args:?}");
    }
    let path = root.path().to_str().unwrap();
    assert!(
        !convert(
            root.path(),
            &["talk.adoc", "--to", "revealjs", "--output", path]
        )
        .status
        .success()
    );
    assert!(!root.path().join("dist").exists());
}

#[test]
fn source_and_include_directories_and_unknown_or_edited_files_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("chapters")).unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\ninclude::chapters/part.adoc[]\n",
    );
    write(root.path(), "chapters/part.adoc", "== Slide\n\nText.\n");
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--include",
            "--to",
            "revealjs",
            "--output",
            "chapters",
        ],
    );
    assert!(!output.status.success());
    assert!(!root.path().join("chapters/index.html").exists());
    success(&convert(
        root.path(),
        &["talk.adoc", "--to", "revealjs", "--output", "dist"],
    ));
    write(root.path(), "dist/manual.txt", "MANUAL");
    assert!(
        !convert(
            root.path(),
            &["talk.adoc", "--to", "revealjs", "--output", "dist"]
        )
        .status
        .success()
    );
    assert_eq!(
        fs::read_to_string(root.path().join("dist/manual.txt")).unwrap(),
        "MANUAL"
    );
    fs::remove_file(root.path().join("dist/manual.txt")).unwrap();
    write(root.path(), "dist/index.html", "HAND_EDITED");
    assert!(
        !convert(
            root.path(),
            &["talk.adoc", "--to", "revealjs", "--output", "dist"]
        )
        .status
        .success()
    );
    assert_eq!(
        fs::read_to_string(root.path().join("dist/index.html")).unwrap(),
        "HAND_EDITED"
    );
}

#[test]
fn body_cannot_link_to_note_only_targets_and_unsafe_svg_cannot_be_saved() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\n<<secret>>\n\n[.notes]\n--\n[#secret]\nPrivate target.\n--\n",
    );
    let output = convert(
        root.path(),
        &["talk.adoc", "--to", "revealjs", "--output", "dist"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("slides-note-only-reference"));
    assert!(!root.path().join("dist").exists());
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\nimage::unsafe.svg[Figure]\n",
    );
    write(
        root.path(),
        "unsafe.svg",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
    );
    let output = convert(
        root.path(),
        &["talk.adoc", "--to", "revealjs", "--output", "dist"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("slides-unsafe-svg"));
    assert!(!root.path().join("dist").exists());
}

#[test]
fn public_reference_ids_are_independent_of_private_notes_and_unrelated_prose_edits() {
    let root = tempfile::tempdir().unwrap();
    let ids = |html: &str| {
        html.split("id=\"")
            .skip(1)
            .map(|part| part.split('"').next().unwrap().to_owned())
            .filter(|id| !id.starts_with("slides-notes-"))
            .collect::<Vec<_>>()
    };
    let mut previous_public = None;
    let mut previous_ids = None;
    for (padding, prose) in [
        ("PRIVATE", "Visible."),
        ("PRIVATE ".repeat(200).as_str(), "Visible."),
        ("PRIVATE", "Unrelated public prose was edited here."),
    ] {
        write(
            root.path(),
            "talk.adoc",
            &format!(
                "= Talk\n\n== First\n\n{prose} cite:[manual] footnote:shared[cite:[manual]].\n\n[.notes]\n--\n{padding} footnote:[Private note]. cite:[manual].\n--\n\n== Last\n\nAgain footnote:shared[] and footnote:[Second]. See xref:#manual[].\n\n[bibliography]\n==== References\n\n* [[[manual]]] Entry.\n"
            ),
        );
        for audience in ["public", "presenter"] {
            success(&convert(
                root.path(),
                &[
                    "--no-config",
                    "talk.adoc",
                    "--to",
                    "revealjs",
                    "--output",
                    audience,
                    "--audience",
                    audience,
                    "--slides-helper",
                    "/missing/helper",
                ],
            ));
            let html = fs::read_to_string(root.path().join(audience).join("index.html")).unwrap();
            check_fragment_targets(&html);
            if let Some(expected) = &previous_ids {
                assert_eq!(&ids(&html), expected);
            } else {
                previous_ids = Some(ids(&html));
            }
            if audience == "public" && prose == "Visible." {
                if let Some(expected) = &previous_public {
                    assert_eq!(&html, expected);
                } else {
                    previous_public = Some(html.clone());
                }
            }
            assert!(
                html.contains("id=\"slides-body-s3-footnote-ref-1\""),
                "{html}"
            );
            assert!(
                html.contains("id=\"slides-body-s3-footnote-ref-2\""),
                "{html}"
            );
            assert!(html.contains("id=\"slides-body-bib-ref-1\""), "{html}");
            assert!(html.contains("id=\"slides-body-bib-ref-2\""), "{html}");
            assert!(
                html.contains("slides-body-s2-footnote-1-bib-ref-1"),
                "{html}"
            );
            assert!(
                html.contains("slides-body-s3-footnote-1-bib-ref-1"),
                "{html}"
            );
        }
    }
}

#[test]
fn leading_notes_do_not_create_public_slides_and_page_titles_use_display_text() {
    let root = tempfile::tempdir().unwrap();
    let body = "== First *bold*\n\nBody.\n";
    write(root.path(), "talk.adoc", body);
    success(&convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "plain",
        ],
    ));
    let plain = fs::read_to_string(root.path().join("plain/index.html")).unwrap();
    write(
        root.path(),
        "talk.adoc",
        &format!("[.notes]\n--\nPRIVATE\n--\n\n{body}"),
    );
    for audience in ["public", "presenter"] {
        success(&convert(
            root.path(),
            &[
                "--no-config",
                "talk.adoc",
                "--to",
                "revealjs",
                "--output",
                audience,
                "--audience",
                audience,
            ],
        ));
        let html = fs::read_to_string(root.path().join(audience).join("index.html")).unwrap();
        assert!(html.contains("<title>First bold</title>"), "{html}");
        assert!(!html.contains("id=\"_preamble\""), "{html}");
        assert_eq!(html.contains("PRIVATE"), audience == "presenter");
        if audience == "public" {
            assert_eq!(html, plain);
        }
    }
    write(
        root.path(),
        "talk.adoc",
        "= Talk *bold* footnote:[title note]\n:topic: resolved\n\n== {topic} _group_\n\n=== Child\n\nBody.\n",
    );
    success(&convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "display",
        ],
    ));
    let html = fs::read_to_string(root.path().join("display/index.html")).unwrap();
    assert!(
        html.contains("<title>Talk bold title note</title>"),
        "{html}"
    );
    assert!(
        html.contains("role=\"group\" aria-label=\"resolved group\""),
        "{html}"
    );
    write(root.path(), "talk.adoc", "[.notes]\n--\nPRIVATE\n--\n");
    let output = convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "empty",
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("slides-empty-deck"));
    assert!(!root.path().join("empty").exists());
    write(root.path(), "talk.adoc", "Visible paragraph.\n");
    success(&convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "fallback",
        ],
    ));
    assert!(
        fs::read_to_string(root.path().join("fallback/index.html"))
            .unwrap()
            .contains("<title>Slides</title>")
    );
}

#[test]
fn hidden_heading_inline_anchors_have_a_specific_source_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\ninclude::part.adoc[]\n\n== Last\n\n<<foo>>\n",
    );
    write(
        root.path(),
        "part.adoc",
        "[%notitle]\n== [[foo]]Hidden\n\nBody.\n",
    );
    let output = convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("part.adoc:2:4: error[slides-hidden-heading-anchor-unsupported]"),
        "{stderr}"
    );
    assert!(stderr.contains("[#id%notitle]"), "{stderr}");
    assert!(!root.path().join("dist").exists());
    success(&convert(root.path(), &["--no-config", "talk.adoc"]));
    write(
        root.path(),
        "part.adoc",
        "[#foo%notitle]\n== Hidden\n\nBody.\n",
    );
    success(&convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
        ],
    ));
    let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
    assert!(html.contains("href=\"#foo\""));
    check_fragment_targets(&html);
}

#[test]
fn private_manual_citations_are_diagnosed_without_preventing_explicit_csl_keys() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\ncite:[shared].\n\n[.notes]\n--\n[bibliography]\n* [[[shared]]] PRIVATE manual entry.\n--\n",
    );
    for audience in ["public", "presenter"] {
        let output = convert(
            root.path(),
            &[
                "--no-config",
                "talk.adoc",
                "--to",
                "revealjs",
                "--output",
                "manual",
                "--audience",
                audience,
            ],
        );
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("talk.adoc:5:1: error[slides-note-only-reference]"),
            "{stderr}"
        );
        assert!(
            !stderr.contains("slides-citation-data-required"),
            "{stderr}"
        );
        assert!(!root.path().join("manual").exists());
    }
    write(
        root.path(),
        "references.json",
        r#"[{"id":"shared","type":"book","title":"Public external work"}]"#,
    );
    write(
        root.path(),
        "style.csl",
        include_str!("../../../packages/slides-helper/fixtures/numeric.csl"),
    );
    write(
        root.path(),
        "locale.xml",
        include_str!("../../../packages/slides-helper/fixtures/locale-en-US.xml"),
    );
    let helper = helper_bin();
    for audience in ["public", "presenter"] {
        success(&convert(
            root.path(),
            &[
                "--no-config",
                "talk.adoc",
                "--to",
                "revealjs",
                "--output",
                audience,
                "--audience",
                audience,
                "--slides-helper",
                &helper,
                "--bibliography",
                "references.json",
                "--csl-style",
                "style.csl",
                "--csl-locale",
                "locale.xml",
            ],
        ));
        let html = fs::read_to_string(root.path().join(audience).join("index.html")).unwrap();
        assert!(html.contains("Public external work"), "{html}");
        assert_eq!(html.matches("<h2>References</h2>").count(), 1, "{html}");
        assert_eq!(html.contains("PRIVATE"), audience == "presenter");
        check_fragment_targets(&html);
    }
}

#[test]
fn included_stem_positional_language_overrides_the_document_setting() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n:stem: asciimath\n\n== Slide\n\ninclude::part.adoc[]\n",
    );
    write(
        root.path(),
        "part.adoc",
        "[stem#energy,tex]\n++++\nE=mc^2\n++++\n",
    );
    let helper = helper_bin();
    success(&convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "latex",
            "--slides-helper",
            &helper,
        ],
    ));
    let html = fs::read_to_string(root.path().join("latex/index.html")).unwrap();
    assert!(html.contains("class=\"math-rendered\""), "{html}");
    assert!(html.contains("id=\"energy\""));
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n:stem: unknown-document-engine\n\n== Slide\n\ninclude::part.adoc[]\n",
    );
    write(
        root.path(),
        "part.adoc",
        "[stem,asciimath]\n++++\nx\n++++\n",
    );
    let output = convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "ascii",
            "--slides-helper",
            "/missing/helper",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("part.adoc:1:1: error[slides-math-unsupported]"),
        "{stderr}"
    );
    assert!(!stderr.contains("stem=unknown-document-engine"), "{stderr}");
    assert!(!root.path().join("ascii").exists());
}

#[test]
fn unsupported_reveal_presentation_metadata_has_original_position_errors() {
    let root = tempfile::tempdir().unwrap();
    for source in [
        "[background-color=yellow]\n== Slide\n",
        "[background-image=missing.png]\n== Slide\n",
        "[background-video=missing.webm]\n== Slide\n",
        "[background-iframe=https://example.test/]\n== Slide\n",
        "[background-size=cover,background-opacity=0.5]\n== Slide\n",
        "[transition=zoom,transition-speed=fast,state=overview,data-custom=value]\n== Slide\n",
        "[%auto-animate]\n== Slide\n",
        "[options=\"auto-animate,auto-animate-restart\"]\n== Slide\n",
        "[.r-fit-text]\nLarge.\n",
        "[.r-stack.r-stretch.stretch]\n--\nContent.\n--\n",
    ] {
        write(root.path(), "talk.adoc", "= Talk\n\ninclude::part.adoc[]\n");
        write(root.path(), "part.adoc", source);
        let output = convert(
            root.path(),
            &[
                "--no-config",
                "talk.adoc",
                "--to",
                "revealjs",
                "--output",
                "dist",
            ],
        );
        assert!(!output.status.success(), "{source}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("part.adoc:1:"), "{stderr}");
        assert!(
            stderr.contains("error[slides-unsupported-option]"),
            "{stderr}"
        );
        assert!(!root.path().join("dist").exists());
        success(&convert(root.path(), &["--no-config", "talk.adoc"]));
    }
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n[.custom%unnumbered,custom=value]\n== Slide\n\n[.custom,background-color=yellow,data-custom=value]\nGeneral metadata.\n",
    );
    success(&convert(
        root.path(),
        &[
            "--no-config",
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
        ],
    ));
}

#[test]
fn unsupported_speaker_note_forms_fail_at_their_original_source() {
    let root = tempfile::tempdir().unwrap();
    for marker in [
        "[NOTE.speaker]",
        "[NOTE.aside]",
        "[NOTE.notes]",
        "[.aside]",
        "[.speaker]",
    ] {
        write(
            root.path(),
            "talk.adoc",
            "= Talk\n\n== Slide\n\ninclude::part.adoc[]\n",
        );
        write(
            root.path(),
            "part.adoc",
            &format!("{marker}\n--\nSECRET\n--\n"),
        );
        for audience in ["public", "presenter"] {
            let output = convert(
                root.path(),
                &[
                    "--no-config",
                    "talk.adoc",
                    "--to",
                    "revealjs",
                    "--output",
                    "dist",
                    "--audience",
                    audience,
                ],
            );
            assert!(!output.status.success(), "{marker}: {audience}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("slides-invalid-notes"), "{stderr}");
            assert!(stderr.contains("part.adoc:"), "{stderr}");
            assert!(!root.path().join("dist").exists());
        }
        let ordinary = convert(root.path(), &["--no-config", "talk.adoc"]);
        success(&ordinary);
        assert!(String::from_utf8_lossy(&ordinary.stdout).contains("SECRET"));
    }
}

#[test]
fn finite_fragment_syntax_rejects_effect_ordering_inline_and_note_steps() {
    let root = tempfile::tempdir().unwrap();
    for (source, expected) in [
        ("[.fragment]\nParagraph.\n", "slides-unsupported-fragment"),
        (
            "[%step,fragment-index=8]\n* Item\n",
            "slides-unsupported-fragment",
        ),
        ("An [.fragment]#inline# effect.\n", "slides-inline-step"),
        ("An [%step]#inline# effect.\n", "slides-inline-step"),
        (
            "[.notes]\n--\n[%step]\n* Note item\n--\n",
            "slides-invalid-step",
        ),
    ] {
        write(
            root.path(),
            "talk.adoc",
            &format!("= Talk\n\n== Slide\n\n{source}"),
        );
        let output = convert(
            root.path(),
            &["talk.adoc", "--to", "revealjs", "--output", "dist"],
        );
        assert!(!output.status.success(), "{source}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!root.path().join("dist").exists());
    }
}

#[cfg(unix)]
#[test]
fn symbolic_links_in_output_ancestors_and_images_are_rejected() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(root.path(), "talk.adoc", "= Talk\n\n== Slide\n\nText.\n");
    symlink(outside.path(), root.path().join("linked")).unwrap();
    assert!(
        !convert(
            root.path(),
            &["talk.adoc", "--to", "revealjs", "--output", "linked/dist"]
        )
        .status
        .success()
    );
    assert!(!outside.path().join("dist").exists());
    write(outside.path(), "plot.svg", SVG);
    symlink(
        outside.path().join("plot.svg"),
        root.path().join("plot.svg"),
    )
    .unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\nimage::plot.svg[Plot]\n",
    );
    assert!(
        !convert(
            root.path(),
            &["talk.adoc", "--to", "revealjs", "--output", "dist"]
        )
        .status
        .success()
    );
    assert!(!root.path().join("dist").exists());
}

fn helper_bin() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/slides-helper/bin.mjs")
        .to_string_lossy()
        .into_owned()
}

fn check_fragment_targets(html: &str) {
    let ids = html
        .split("id=\"")
        .skip(1)
        .map(|part| part.split('"').next().unwrap())
        .collect::<Vec<_>>();
    let unique = ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), unique.len(), "duplicate IDs in {html}");
    for part in html.split("href=\"#").skip(1) {
        let target = part.split('"').next().unwrap();
        assert!(
            unique.contains(target),
            "missing fragment target {target} in {html}"
        );
    }
}

#[test]
fn shared_footnote_math_and_citations_use_first_reference_order_and_safe_placement_ids() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        concat!(
            "= Talk\n:eqnums:\n\n== First\n\ncite:[before]. First footnote:shared[]. Then cite:[after].\n\n",
            "[.notes]\n--\nNotes footnote:shared[].\n--\n\n",
            "== Last\n\nDefinition footnote:shared[Forward latexmath:[\\eqref{inside}] and latexmath:[\\begin{equation}x=1\\label{inside}\\end{equation}], cite:[inside].]. External latexmath:[\\eqref{inside}].\n\n",
            "[.notes]\n--\nAgain footnote:shared[].\n--\n"
        ),
    );
    write(
        root.path(),
        "references.json",
        r#"[{"id":"before","title":"Before"},{"id":"inside","title":"Inside"},{"id":"after","title":"After"}]"#,
    );
    write(
        root.path(),
        "style.csl",
        include_str!("../../../packages/slides-helper/fixtures/numeric.csl"),
    );
    write(
        root.path(),
        "locale.xml",
        include_str!("../../../packages/slides-helper/fixtures/locale-en-US.xml"),
    );
    let helper = helper_bin();
    let args = [
        "talk.adoc",
        "--to",
        "revealjs",
        "--output",
        "dist",
        "--audience",
        "presenter",
        "--slides-helper",
        &helper,
        "--bibliography",
        "references.json",
        "--csl-style",
        "style.csl",
        "--csl-locale",
        "locale.xml",
    ];
    success(&convert(root.path(), &args));
    let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
    check_fragment_targets(&html);
    assert!(!html.contains("<code class=\"math-latex\""), "{html}");
    assert_eq!(html.matches("class=\"math-rendered\"").count(), 9, "{html}");
    assert!(
        html.contains("slides-body-s2-footnote-1-body-m1-i"),
        "{html}"
    );
    assert!(
        html.contains("slides-body-s3-footnote-1-body-m1-i"),
        "{html}"
    );
    assert!(
        html.contains("slides-notes-s2-footnote-1-notes-m1-i"),
        "{html}"
    );
    let bibliography = html.split("id=\"slides-body-references\"").nth(1).unwrap();
    let before = bibliography.find("slides-body-bib-before").unwrap();
    let inside = bibliography.find("slides-body-bib-inside").unwrap();
    let after = bibliography.find("slides-body-bib-after").unwrap();
    assert!(before < inside && inside < after, "{bibliography}");
    assert!(
        bibliography.contains("href=\"#slides-body-s2-footnote-1-bib-ref-"),
        "{bibliography}"
    );
    assert!(
        bibliography.contains("href=\"#slides-body-s3-footnote-1-bib-ref-"),
        "{bibliography}"
    );
    assert_eq!(html.matches("id=\"slides-body-bib-inside\"").count(), 1);
    assert_eq!(html.matches("id=\"slides-notes-bib-inside\"").count(), 1);
    assert!(
        root.path()
            .join("dist/licenses/mathjax-license.txt")
            .exists()
    );
    assert!(
        root.path()
            .join("dist/licenses/citations-license.txt")
            .exists()
    );
}

#[test]
fn manual_only_citations_in_shared_footnotes_need_no_helper_or_external_library() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== First\n\nFirst footnote:shared[cite:[manual].].\n\n[bibliography]\n==== References\n\n* [[[manual]]] Hand-written entry.\n\n== Last\n\nAgain footnote:shared[].\n\n[.notes]\n--\nNotes footnote:shared[].\n--\n",
    );
    success(&convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--audience",
            "presenter",
            "--slides-helper",
            "/missing/helper",
        ],
    ));
    let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
    check_fragment_targets(&html);
    assert_eq!(html.matches("href=\"#manual\"").count(), 3, "{html}");
    assert!(
        html.contains("href=\"#slides-body-s2-footnote-1-bib-ref-"),
        "{html}"
    );
    assert!(
        html.contains("href=\"#slides-body-s3-footnote-1-bib-ref-"),
        "{html}"
    );
    assert!(!html.contains("slides-body-references"));
    assert!(
        !root
            .path()
            .join("dist/licenses/citations-license.txt")
            .exists()
    );
}

#[test]
fn plain_footnotes_and_public_note_only_dependencies_are_never_acquired() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== First\n\nPlain footnote:[Shared *detail*].\n\n.Private latexmath:[x] cite:[secret]\n[.notes]\n--\nNotes footnote:[anchor:private[] latexmath:[y] cite:[secret].].\n--\n",
    );
    success(&convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--slides-helper",
            "/missing/helper",
            "--math-macros",
            "missing-macros.json",
            "--bibliography",
            "missing.json",
            "--csl-style",
            "missing.csl",
            "--csl-locale",
            "missing.xml",
        ],
    ));
    let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
    assert!(html.contains("Shared <strong>detail</strong>"));
    assert!(!html.contains("secret"), "{html}");
    assert!(!html.contains("licenses/mathjax-license.txt"));
}

#[test]
fn footnote_only_macros_are_loaded_and_helper_errors_keep_the_included_definition_location() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\ninclude::part.adoc[]\n",
    );
    write(
        root.path(),
        "part.adoc",
        "First footnote:shared[latexmath:[\\custom]].\n",
    );
    write(
        root.path(),
        "macros.json",
        r#"[{"name":"custom","definition":"x+1"}]"#,
    );
    let helper = helper_bin();
    success(&convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--slides-helper",
            &helper,
            "--math-macros",
            "macros.json",
            "--bibliography",
            "missing.json",
            "--csl-style",
            "missing.csl",
            "--csl-locale",
            "missing.xml",
        ],
    ));
    let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
    check_fragment_targets(&html);
    assert_eq!(html.matches("class=\"math-rendered\"").count(), 1);
    write(
        root.path(),
        "part.adoc",
        "First footnote:shared[latexmath:[\\undefinedAdocWeaveMacro]].\n",
    );
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "failed",
            "--slides-helper",
            &helper,
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("part.adoc:1:"), "{stderr}");
    assert!(stderr.contains("Undefined control sequence"), "{stderr}");
    assert!(!root.path().join("failed").exists());
}

#[test]
fn footnote_citation_errors_are_checked_before_library_reads_and_save() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\nFirst footnote:[cite:[absent]].\n",
    );
    let output = convert(
        root.path(),
        &["talk.adoc", "--to", "revealjs", "--output", "dist"],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("talk.adoc:5:"), "{stderr}");
    assert!(stderr.contains("slides-citation-data-required"), "{stderr}");
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\nFirst footnote:[cite:[manual]].\n\n[bibliography]\n==== References\n\n* [[[manual]]] Entry.\n",
    );
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--bibliography",
            "missing.json",
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("slides-bibliography-key-conflict"),
        "{stderr}"
    );
    assert!(!stderr.contains("cannot open"), "{stderr}");
    assert!(!root.path().join("dist").exists());
}

#[test]
fn caption_footnote_equations_belong_to_their_visible_owner() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\n.Caption footnote:[latexmath:[x]].\nimage::figure.svg[Figure]\n\n.Private footnote:[latexmath:[y] cite:[private]].\n[.notes]\n--\nPrivate.\n--\n",
    );
    write(root.path(), "figure.svg", SVG);
    let helper = helper_bin();
    success(&convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--slides-helper",
            &helper,
            "--bibliography",
            "missing.json",
            "--csl-style",
            "missing.csl",
            "--csl-locale",
            "missing.xml",
        ],
    ));
    let html = fs::read_to_string(root.path().join("dist/index.html")).unwrap();
    check_fragment_targets(&html);
    assert_eq!(html.matches("class=\"math-rendered\"").count(), 1, "{html}");
    assert!(html.contains("Figure 1. Caption"), "{html}");
    assert!(!html.contains("Private"));
    assert!(!html.contains("private"));
}

#[test]
fn copied_footnote_math_ids_are_checked_against_authored_ids_before_save() {
    let root = tempfile::tempdir().unwrap();
    let source = "= Talk\n\n== Slide\n\nFirst footnote:[latexmath:[x]].\n";
    write(root.path(), "talk.adoc", source);
    let helper = helper_bin();
    success(&convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "valid",
            "--slides-helper",
            &helper,
        ],
    ));
    let html = fs::read_to_string(root.path().join("valid/index.html")).unwrap();
    let math_id = html
        .split("id=\"")
        .skip(1)
        .map(|part| part.split('"').next().unwrap())
        .find(|id| id.starts_with("slides-body-s2-footnote-1-body-m0-i"))
        .unwrap();
    write(
        root.path(),
        "talk.adoc",
        &format!("{source}\n[#{math_id}]\n--\nAuthored target.\n--\n"),
    );
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "failed",
            "--slides-helper",
            &helper,
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("slides-generated-id-collision"), "{stderr}");
    assert!(stderr.contains(math_id), "{stderr}");
    assert!(!root.path().join("failed").exists());
}

#[test]
fn authored_anchor_definitions_inside_footnotes_fail_before_helper_or_data_acquisition() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "talk.adoc",
        "= Talk\n\n== Slide\n\nFirst footnote:shared[anchor:inside[] latexmath:[x]].\n\n== Again\n\nAgain footnote:shared[].\n",
    );
    let output = convert(
        root.path(),
        &[
            "talk.adoc",
            "--to",
            "revealjs",
            "--output",
            "dist",
            "--slides-helper",
            "/missing/helper",
            "--math-macros",
            "missing.json",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("slides-footnote-anchor-unsupported"),
        "{stderr}"
    );
    assert!(stderr.contains("talk.adoc:5:"), "{stderr}");
    assert!(!stderr.contains("missing.json"), "{stderr}");
    assert!(!root.path().join("dist").exists());
}
