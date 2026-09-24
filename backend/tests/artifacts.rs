//! I-ART: the object store, against a real MinIO (`TEST_S3_ENDPOINT`). Each
//! test gets a bucket of its own and removes it when it ends.

mod common;

use birdtest::jobs::leave_gen::artifact_key;
use common::TestDb;
use uuid::Uuid;

/// I-ART-1: what goes in comes out, byte for byte -- including bytes that are
/// not text, and an object big enough that a truncated read would show.
#[tokio::test]
async fn put_then_get_returns_identical_bytes() {
    let db = TestDb::new().await;
    let (state, bucket) = db.state_with_object_store().await;
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let key = artifact_key(Uuid::new_v4(), 3);
    assert_eq!(state.artifacts.put(&key, bytes.clone()).await.unwrap(), key);
    assert!(state.artifacts.exists(&key).await.unwrap());
    assert_eq!(state.artifacts.get(&key).await.unwrap(), bytes);
    assert_eq!(bucket.keys().await, vec![key]);
}

/// I-ART-2: an absent key is an error the caller can act on -- a 404, and
/// `exists` says no -- rather than a panic or a 500.
#[tokio::test]
async fn getting_an_absent_key_is_a_clean_not_found() {
    let db = TestDb::new().await;
    let (state, _bucket) = db.state_with_object_store().await;
    let key = artifact_key(Uuid::new_v4(), 1);
    assert!(!state.artifacts.exists(&key).await.unwrap());
    let err = state.artifacts.get(&key).await.unwrap_err();
    assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    assert!(err.message.contains(&key), "{}", err.message);
}

/// I-ART-3: keys are namespaced by job and generation, so two jobs -- or two
/// generations of one -- writing at once cannot overwrite each other.
#[tokio::test]
async fn two_jobs_and_two_generations_never_share_a_key() {
    let db = TestDb::new().await;
    let (state, bucket) = db.state_with_object_store().await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let writes = [(a, 1, b"a1".to_vec()), (a, 2, b"a2".to_vec()), (b, 1, b"b1".to_vec())];
    for (job, generation, body) in &writes {
        state.artifacts.put(&artifact_key(*job, *generation), body.clone()).await.unwrap();
    }
    for (job, generation, body) in &writes {
        let key = artifact_key(*job, *generation);
        assert!(key.contains(&job.to_string()) && key.contains(&format!("generation-{generation}")));
        assert_eq!(&state.artifacts.get(&key).await.unwrap(), body);
    }
    assert_eq!(bucket.keys().await.len(), 3);
}

/// The bucket really is removed with its test: nothing a test writes outlives
/// it.
#[tokio::test]
async fn a_test_bucket_is_removed_with_everything_in_it() {
    let db = TestDb::new().await;
    let (state, bucket) = db.state_with_object_store().await;
    state.artifacts.put("leftover", vec![1, 2, 3]).await.unwrap();
    let name = bucket.name.clone();
    drop(bucket);
    assert!(state.artifacts.get("leftover").await.is_err(), "{name} kept its object");
    // Writing into it fails too: the bucket itself is gone, not just emptied.
    let err = state.artifacts.put("again", vec![4]).await.unwrap_err();
    assert!(err.message.contains("S3 put again failed"), "{name}: {}", err.message);
}
