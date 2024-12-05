use test::common::redis_setup;
use users::mem_redis::{delete_entry, get_entry, get_phantom_token, set_entry, set_entry_timeout};

use super::super::*;

#[tokio::test]
async fn test_setup() {
    assert!(redis_setup().await.is_ok());
}

#[tokio::test]
async fn test_set_entry() {
    let mut pool = redis_setup().await.unwrap();

    assert!(
        set_entry(&mut pool, &"test".to_string(), "test".to_string())
            .await
            .is_ok()
    );

    let res = delete_entry(&mut pool, &"test".to_string()).await;

    assert!(res.is_ok());
}

#[tokio::test]
async fn test_set_entry_timeout() {
    let mut pool = redis_setup().await.unwrap();

    assert!(
        set_entry(&mut pool, &"test".to_string(), "test".to_string())
            .await
            .is_ok()
    );

    assert!(set_entry_timeout(&mut pool, &"test".to_string(), 1)
        .await
        .is_ok());

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let res = get_entry(&mut pool, &"test".to_string()).await;

    assert!(res.is_err());
}

#[tokio::test]
async fn test_get_phantom_token() {
    let id = "test".to_string();

    assert_eq!(get_phantom_token(&id), "phantom_test");
}

#[tokio::test]
async fn test_delete_entry() {
    let mut pool = redis_setup().await.unwrap();

    assert!(
        set_entry(&mut pool, &"test".to_string(), "test".to_string())
            .await
            .is_ok()
    );

    let res = delete_entry(&mut pool, &"test".to_string()).await;

    assert!(res.is_ok(), "Delete first time. Res is `{:?}`", res);

    let res = get_entry(&mut pool, &"test".to_string()).await;

    assert!(res.is_err(), "Retrieve entry. Res is `{:?}`", res);

    let res = delete_entry(&mut pool, &"test".to_string()).await;

    assert!(res.is_ok(), "Delete again. Res is `{:?}`", res);
}
