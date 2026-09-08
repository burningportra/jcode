use jcode_storage::{
    StorageRecoveryEvent, read_json_with_recovery_handler, upsert_env_file_value, write_json,
};
use serde_json::{Value, json};

#[test]
fn env_update_and_removal_preserve_unreadable_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("provider.env");
    let original = b"OTHER_KEY=keep-me\nINVALID=\xff\n";
    let backup = path.with_extension("bak");
    std::fs::write(&path, original).unwrap();
    std::fs::write(&backup, "previous contents").unwrap();

    for value in [Some("replacement-secret"), None] {
        let error = upsert_env_file_value(&path, "API_KEY", value).unwrap_err();
        assert!(error.to_string().contains(&path.display().to_string()));
        assert!(!format!("{error:#}").contains("replacement-secret"));
        assert!(error.downcast_ref::<std::io::Error>().is_some());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "previous contents"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

#[test]
fn env_update_rejects_directory_without_renaming_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("provider.env");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("keep"), "untouched").unwrap();

    assert!(upsert_env_file_value(&path, "API_KEY", Some("new")).is_err());
    assert_eq!(
        std::fs::read_to_string(path.join("keep")).unwrap(),
        "untouched"
    );
    assert!(!path.with_extension("bak").exists());
}

#[test]
fn env_update_creates_replaces_and_removes_only_requested_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("provider.env");
    upsert_env_file_value(&path, "API_KEY", Some("initial")).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "API_KEY=initial\n");
    std::fs::write(&path, "# comment\nOTHER_KEY=keep\nAPI_KEY=initial\n").unwrap();

    upsert_env_file_value(&path, "API_KEY", Some("updated")).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# comment\nOTHER_KEY=keep\nAPI_KEY=updated\n"
    );
    upsert_env_file_value(&path, "API_KEY", None).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# comment\nOTHER_KEY=keep\n"
    );
}

#[test]
fn valid_primary_wins_without_recovery_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    write_json(&path, &json!({"revision": 1})).unwrap();
    write_json(&path, &json!({"revision": 2})).unwrap();

    let value: Value = read_json_with_recovery_handler(&path, |_| {
        panic!("valid primary must not trigger recovery")
    })
    .unwrap();
    assert_eq!(value, json!({"revision": 2}));
}

#[test]
fn backup_recovery_preserves_primary_and_backup_bytes() {
    for corrupt in [b"{broken".as_slice(), b"{\"text\":\"\xff\"}".as_slice()] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let backup = path.with_extension("bak");
        let backup_bytes = b"{\"revision\": 1}\n";
        std::fs::write(&path, corrupt).unwrap();
        std::fs::write(&backup, backup_bytes).unwrap();
        let mut events = Vec::new();

        let value: Value = read_json_with_recovery_handler(&path, |event| match event {
            StorageRecoveryEvent::CorruptPrimary { path: failed, .. } => {
                assert_eq!(failed, path);
                events.push("corrupt");
            }
            StorageRecoveryEvent::RecoveredFromBackup { backup_path } => {
                assert_eq!(backup_path, backup);
                events.push("recovered");
            }
        })
        .unwrap();

        assert_eq!(value, json!({"revision": 1}));
        assert_eq!(events, ["corrupt", "recovered"]);
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        assert_eq!(std::fs::read(&backup).unwrap(), backup_bytes);
    }
}

#[test]
fn backup_recovery_does_not_overwrite_newer_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    std::fs::write(&path, "{broken").unwrap();
    std::fs::write(path.with_extension("bak"), "{\"revision\":1}").unwrap();

    let recovered: Value = read_json_with_recovery_handler(&path, |event| {
        if matches!(event, StorageRecoveryEvent::RecoveredFromBackup { .. }) {
            // Interleave a real atomic save after decoding the backup, before returning.
            write_json(&path, &json!({"revision": 2})).unwrap();
        }
    })
    .unwrap();

    assert_eq!(recovered, json!({"revision": 1}));
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved, json!({"revision": 2}));
}

#[test]
fn corrupt_primary_without_valid_backup_returns_error_without_writes() {
    for backup_contents in [
        None,
        Some(b"{also broken".as_slice()),
        Some(b"\xff".as_slice()),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let backup = path.with_extension("bak");
        std::fs::write(&path, "{broken").unwrap();
        if let Some(contents) = backup_contents {
            std::fs::write(&backup, contents).unwrap();
        }
        let result = read_json_with_recovery_handler::<Value, _>(&path, |event| {
            assert!(matches!(event, StorageRecoveryEvent::CorruptPrimary { .. }));
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        if let Some(contents) = backup_contents {
            assert_eq!(std::fs::read(&backup).unwrap(), contents);
        } else {
            assert!(!backup.exists());
        }
    }
}

#[test]
fn missing_primary_is_not_resurrected_from_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    std::fs::write(path.with_extension("bak"), "{\"revision\":1}").unwrap();
    let error = read_json_with_recovery_handler::<Value, _>(&path, |_| {
        panic!("missing files must not trigger corruption recovery")
    })
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(!path.exists());
}
