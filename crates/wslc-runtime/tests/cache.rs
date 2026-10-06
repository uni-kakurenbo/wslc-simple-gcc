#![cfg(windows)]

use sha2::{Digest, Sha256};
use std::fs;
use wslc_runtime::{sdk, session};

#[test]
fn verified_cache_and_exclusive_lock_fail_closed_and_release() {
    let root = std::env::temp_dir().join(format!("wslc-cache-test-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("dependency");
    fs::write(&path, "verified bytes").unwrap();
    let digest = format!("{:x}", Sha256::digest(b"verified bytes"));
    sdk::verify(&path, &digest.to_uppercase()).unwrap();

    fs::write(&path, "corrupt bytes").unwrap();
    let error = sdk::fetch("invalid.example", "/not-requested", &path, &digest).unwrap_err();
    assert!(error.contains("Checksum mismatch"));
    assert!(
        sdk::verify(&path, "bad digest")
            .unwrap_err()
            .contains("SHA-256")
    );

    let lock = session::lock(&root).unwrap();
    assert!(session::lock(&root).is_err());
    drop(lock);
    drop(session::lock(&root).unwrap());
    fs::remove_dir_all(root).unwrap();
}
