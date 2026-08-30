use std::collections::BTreeSet;

fn direct_dependencies(manifest: &str) -> BTreeSet<String> {
    toml::from_str::<toml::Table>(manifest)
        .expect("valid Cargo manifest")
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .map(|dependencies| dependencies.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn project_has_only_lower_level_crates_and_standard_glob_dependency() {
    let actual = direct_dependencies(include_str!("../Cargo.toml"));
    let expected = BTreeSet::from([
        "adocweave".to_owned(),
        "glob".to_owned(),
        "serde".to_owned(),
        "sha2".to_owned(),
        "toml".to_owned(),
    ]);
    assert_eq!(actual, expected);
}

#[test]
fn lower_level_crates_do_not_depend_on_project() {
    let manifest = include_str!("../../adocweave/Cargo.toml");
    assert!(
        !direct_dependencies(manifest).contains("adocweave-project"),
        "adocweave must remain below adocweave-project"
    );
}

#[test]
fn project_contract_adds_no_service_or_runtime_abstraction() {
    let source = include_str!("../src/lib.rs");
    for forbidden in [
        "pub trait ",
        "dyn FileSystem",
        "async fn",
        "tokio",
        "serde",
        "Mutex",
        "RwLock",
        "WorkspaceService",
        "todo!",
        "unimplemented!",
        "NotImplemented",
        "ProjectDependency",
    ] {
        assert!(
            !source.contains(forbidden),
            "project contract contains forbidden abstraction: {forbidden}"
        );
    }
}

#[test]
fn public_contract_does_not_name_lower_layer_types() {
    let source = include_str!("../src/lib.rs");
    for line in source
        .lines()
        .filter(|line| line.trim_start().starts_with("pub "))
    {
        let tokens = line
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .collect::<BTreeSet<_>>();
        for forbidden in [
            "FilesystemError",
            "FilesystemReadLimits",
            "FilesystemAuthority",
            "adocweave_workspace",
        ] {
            assert!(
                !tokens.contains(forbidden),
                "public contract leaks {forbidden}: {line}"
            );
        }
    }
}
