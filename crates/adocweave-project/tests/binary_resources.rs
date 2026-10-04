use std::fs;

use adocweave_core::{CancellationToken, NeverCancel};
use adocweave_project::{ProjectAuthority, ProjectObservationKind, ProjectResourceLimits};

#[test]
fn binary_observation_detects_same_size_changes_without_decoding_utf8() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("figure.png");
    fs::write(&path, [0, 1, 255]).unwrap();
    let authority = ProjectAuthority::open(root.path(), [root.path().to_owned()]).unwrap();
    let resource = authority
        .read_binary_resources(
            &[root.path().to_owned()],
            std::slice::from_ref(&path),
            ProjectResourceLimits::default(),
            &NeverCancel,
        )
        .unwrap()
        .remove(0);
    let acquired = resource.observation();
    assert_eq!(
        acquired.kind,
        ProjectObservationKind::BinaryContentsNoSymlinks
    );
    let access = authority.observation_access();
    assert_eq!(
        access.observer().observe(&path, acquired.kind),
        acquired.observation
    );
    fs::write(&path, [0, 2, 255]).unwrap();
    assert_ne!(
        access.observer().observe(&path, acquired.kind),
        acquired.observation
    );
    fs::remove_file(&path).unwrap();
    assert_ne!(
        access.observer().observe(&path, acquired.kind),
        acquired.observation
    );
}

#[test]
fn selected_binary_reads_are_deduplicated_confined_and_bounded() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(root.path().join("one.png"), [0, 1, 255]).unwrap();
    fs::write(root.path().join("two.png"), [2, 3, 254]).unwrap();
    fs::write(outside.path().join("private.png"), [9]).unwrap();
    let authority = ProjectAuthority::open(root.path(), [root.path().to_owned()]).unwrap();
    let limits = ProjectResourceLimits {
        max_files: 1,
        max_resource_bytes: 3,
        max_total_bytes: 3,
    };
    let path = root.path().join("one.png");
    let resources = authority
        .read_binary_resources(
            &[root.path().to_owned()],
            &[path.clone(), path.clone()],
            limits,
            &NeverCancel,
        )
        .unwrap();
    assert!(
        authority
            .read_binary_resources(
                &[root.path().join("unused-private-root")],
                &[],
                limits,
                &NeverCancel
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].bytes, [0, 1, 255]);
    assert_eq!(resources[0].sha256().len(), 64);
    assert!(
        authority
            .read_binary_resources(
                &[root.path().to_owned()],
                &[path.clone(), root.path().join("two.png")],
                limits,
                &NeverCancel
            )
            .is_err()
    );
    assert!(
        authority
            .read_binary_resources(
                &[root.path().to_owned()],
                std::slice::from_ref(&path),
                ProjectResourceLimits {
                    max_total_bytes: 2,
                    ..limits
                },
                &NeverCancel
            )
            .is_err()
    );
    assert!(
        authority
            .read_binary_resources(
                &[root.path().to_owned()],
                &[outside.path().join("private.png")],
                limits,
                &NeverCancel
            )
            .is_err()
    );
    assert!(
        authority
            .read_binary_resources(
                &[outside.path().to_owned()],
                &[outside.path().join("private.png")],
                limits,
                &NeverCancel
            )
            .is_err()
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(
        authority
            .read_binary_resources(&[root.path().to_owned()], &[path], limits, &cancellation)
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn symbolic_link_binary_resources_never_use_outside_bytes() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("private.png"), b"PRIVATE").unwrap();
    symlink(
        outside.path().join("private.png"),
        root.path().join("image.png"),
    )
    .unwrap();
    let authority = ProjectAuthority::open(root.path(), [root.path().to_owned()]).unwrap();
    assert!(
        authority
            .read_binary_resources(
                &[root.path().to_owned()],
                &[root.path().join("image.png")],
                ProjectResourceLimits::default(),
                &NeverCancel
            )
            .is_err()
    );
}
