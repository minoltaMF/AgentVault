use std::str::FromStr;
use std::sync::{Arc, Barrier};

use vault::{ObjectDisposition, ObjectId, ObjectStore, ObjectStoreError, MAX_OBJECT_BYTES};

#[test]
fn put_bytes_uses_sha256_fanout_and_round_trips_compressed_content() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&objects_root).expect("objects root");
    let store = ObjectStore::open(&objects_root).expect("safe objects root");

    let stored = store.put_bytes(b"hello").expect("store object");
    let expected = ObjectId::from_str(
        "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    )
    .expect("known digest");
    let expected_path = objects_root
        .join("sha256")
        .join("2c")
        .join("f24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");

    assert_eq!(stored.id(), &expected);
    assert_eq!(stored.logical_size(), 5);
    assert_eq!(stored.disposition(), ObjectDisposition::Created);
    assert_eq!(store.object_path(&expected), expected_path);
    assert_ne!(
        std::fs::read(expected_path).expect("encoded object"),
        b"hello"
    );
    assert_eq!(store.read(&expected).expect("read object"), b"hello");
}

#[test]
fn repeated_put_reuses_the_verified_object_without_rewriting_it() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&objects_root).expect("objects root");
    let store = ObjectStore::open(&objects_root).expect("safe objects root");
    let first = store.put_bytes(b"same content").expect("first write");
    let object_path = store.object_path(first.id());
    let encoded = std::fs::read(&object_path).expect("first encoded object");

    let second = store
        .put_bytes(b"same content")
        .expect("deduplicated write");

    assert_eq!(second.id(), first.id());
    assert_eq!(second.disposition(), ObjectDisposition::Reused);
    assert_eq!(
        std::fs::read(object_path).expect("unchanged encoded object"),
        encoded
    );
}

#[test]
fn concurrent_puts_create_once_and_reuse_the_same_object() {
    const WRITERS: usize = 8;

    let temp = tempfile::tempdir().expect("temporary vault");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&objects_root).expect("objects root");
    let store = Arc::new(ObjectStore::open(&objects_root).expect("safe objects root"));
    let content = Arc::new(vec![b'x'; 1024 * 1024]);
    let barrier = Arc::new(Barrier::new(WRITERS));

    let writers = (0..WRITERS)
        .map(|_| {
            let store = Arc::clone(&store);
            let content = Arc::clone(&content);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.put_bytes(content.as_slice())
            })
        })
        .collect::<Vec<_>>();
    let stored = writers
        .into_iter()
        .map(|writer| writer.join().expect("writer thread").expect("store object"))
        .collect::<Vec<_>>();

    assert_eq!(
        stored
            .iter()
            .filter(|object| object.disposition() == ObjectDisposition::Created)
            .count(),
        1
    );
    assert_eq!(
        stored
            .iter()
            .filter(|object| object.disposition() == ObjectDisposition::Reused)
            .count(),
        WRITERS - 1
    );
    assert!(stored.iter().all(|object| object.id() == stored[0].id()));
    assert_eq!(
        store.read(stored[0].id()).expect("read shared object"),
        content.as_slice()
    );
}

#[test]
fn corrupt_existing_object_is_reported_and_never_overwritten() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&objects_root).expect("objects root");
    let store = ObjectStore::open(&objects_root).expect("safe objects root");
    let stored = store.put_bytes(b"protected content").expect("first write");
    let object_path = store.object_path(stored.id());
    let corrupt = b"corrupt object envelope";
    std::fs::write(&object_path, corrupt).expect("corrupt test object");

    let error = store
        .put_bytes(b"protected content")
        .expect_err("corrupt object must not be reused");

    assert!(matches!(error, ObjectStoreError::InvalidEnvelope(_)));
    assert_eq!(
        std::fs::read(object_path).expect("corrupt object preserved"),
        corrupt
    );
}

#[test]
fn valid_envelope_with_wrong_content_is_rejected_by_digest() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&objects_root).expect("objects root");
    let store = ObjectStore::open(&objects_root).expect("safe objects root");
    let expected = store.put_bytes(b"expected").expect("expected object");
    let replacement = store.put_bytes(b"replacement").expect("replacement object");
    let expected_path = store.object_path(expected.id());
    let replacement_bytes =
        std::fs::read(store.object_path(replacement.id())).expect("replacement bytes");
    std::fs::write(&expected_path, &replacement_bytes).expect("replace test object");

    let error = store
        .read(expected.id())
        .expect_err("path and content digest differ");

    assert!(matches!(
        error,
        ObjectStoreError::ObjectDigestMismatch { expected: actual_expected, actual }
            if actual_expected == *expected.id() && actual == *replacement.id()
    ));
    assert_eq!(
        std::fs::read(expected_path).expect("mismatched object preserved"),
        replacement_bytes
    );
}

#[test]
fn oversized_objects_are_rejected_before_any_object_file_is_created() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&objects_root).expect("objects root");
    let store = ObjectStore::open(&objects_root).expect("safe objects root");

    let bytes = vec![0_u8; MAX_OBJECT_BYTES + 1];
    let error = store.put_bytes(&bytes).expect_err("object must be bounded");

    assert!(matches!(
        error,
        ObjectStoreError::ObjectTooLarge {
            size,
            max
        } if size == (MAX_OBJECT_BYTES + 1) as u64 && max == MAX_OBJECT_BYTES as u64
    ));
    assert_eq!(
        std::fs::read_dir(objects_root.join("sha256"))
            .expect("sha256 namespace")
            .count(),
        0
    );
}

#[test]
fn oversized_encoded_object_is_rejected_before_reading_it_into_memory() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&objects_root).expect("objects root");
    let store = ObjectStore::open(&objects_root).expect("safe objects root");
    let stored = store.put_bytes(b"small object").expect("store object");
    let object_path = store.object_path(stored.id());
    let oversized = (MAX_OBJECT_BYTES + 2 * 1024 * 1024) as u64;
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&object_path)
        .expect("open test object")
        .set_len(oversized)
        .expect("make sparse oversized object");

    let error = store
        .read(stored.id())
        .expect_err("encoded object must be bounded");

    assert!(matches!(
        error,
        ObjectStoreError::EncodedObjectTooLarge { size, max }
            if size == oversized && max < oversized
    ));
}
