use std::collections::BTreeSet;

fn direct_dependencies(manifest: &str) -> BTreeSet<&str> {
    manifest
        .split_once("[dependencies]\n")
        .map(|(_, dependencies)| dependencies)
        .unwrap_or_default()
        .split("\n[")
        .next()
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name.trim()))
        .filter(|name| !name.is_empty())
        .collect()
}

#[test]
fn project_has_only_the_three_lower_level_crate_dependencies() {
    let actual = direct_dependencies(include_str!("../Cargo.toml"));
    let expected = BTreeSet::from(["adocweave", "adocweave-config", "adocweave-host"]);
    assert_eq!(actual, expected);
}

#[test]
fn lower_level_crates_do_not_depend_on_project() {
    for (name, manifest) in [
        ("adocweave", include_str!("../../adocweave/Cargo.toml")),
        (
            "adocweave-config",
            include_str!("../../adocweave-config/Cargo.toml"),
        ),
        (
            "adocweave-host",
            include_str!("../../adocweave-host/Cargo.toml"),
        ),
    ] {
        assert!(
            !direct_dependencies(manifest).contains("adocweave-project"),
            "{name} must remain below adocweave-project"
        );
    }
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
    ] {
        assert!(
            !source.contains(forbidden),
            "project contract contains forbidden abstraction: {forbidden}"
        );
    }
}
