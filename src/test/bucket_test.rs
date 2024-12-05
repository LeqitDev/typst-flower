use common::bucket_setup;
use users::bucket::{delete_object, get_object, get_object_list, put_object};

use super::super::*;
use super::*;

#[tokio::test]
async fn test_setup() {
    assert!(bucket_setup().await.is_ok());
}

#[tokio::test]
async fn test_put_object() {
    let client = bucket_setup().await.unwrap();
    let bucket = env::var("MINIO_BUCKET").unwrap();

    assert!(put_object(
        &client,
        &bucket,
        "test/flower.typ".to_string(),
        "test".to_string()
    )
    .await
    .is_ok());

    let res = delete_object(&client, &bucket, "test/flower.typ".to_string()).await;

    assert!(res.is_ok());
}

#[tokio::test]
async fn test_get_object_list() {
    let client = bucket_setup().await.unwrap();
    let bucket = env::var("MINIO_BUCKET").unwrap();

    assert!(put_object(
        &client,
        &bucket,
        "test/list/flower.typ".to_string(),
        "test".to_string()
    )
    .await
    .is_ok());

    let res = get_object_list(&client, &bucket, "test/list".to_string()).await;

    assert!(res.is_ok());

    let list = res.unwrap();

    assert_eq!(list.len(), 1);

    assert_eq!(list[0].name, "test/list/flower.typ");

    let res = delete_object(&client, &bucket, "test/list/flower.typ".to_string()).await;

    assert!(res.is_ok());
}

#[tokio::test]
async fn test_get_object() {
    let client = bucket_setup().await.unwrap();
    let bucket = env::var("MINIO_BUCKET").unwrap();

    assert!(put_object(
        &client,
        &bucket,
        "test/get.typ".to_string(),
        "test".to_string()
    )
    .await
    .is_ok());

    let res = get_object(&client, &bucket, "test/get.typ".to_string()).await;

    assert!(res.is_ok());

    assert_eq!(res.unwrap(), "test");

    let res = delete_object(&client, &bucket, "test/get.typ".to_string()).await;

    assert!(res.is_ok());
}

#[tokio::test]
async fn test_get_object_fail() {
    let client = bucket_setup().await.unwrap();
    let bucket = env::var("MINIO_BUCKET").unwrap();

    let res = get_object(&client, &bucket, "test/flower.typ2".to_string()).await;

    assert!(res.is_err());
}

#[tokio::test]
async fn test_get_object_list_fail() {
    let client = bucket_setup().await.unwrap();
    let bucket = env::var("MINIO_BUCKET").unwrap();

    let res = get_object_list(&client, &bucket, "test2".to_string()).await;

    assert!(res.is_ok() && res.unwrap().is_empty());
}

#[tokio::test]
async fn remove_object() {
    let client = bucket_setup().await.unwrap();
    let bucket = env::var("MINIO_BUCKET").unwrap();

    let res = delete_object(&client, &bucket, "test/rem.typ".to_string()).await;

    assert!(res.is_ok() && !res.unwrap().is_delete_marker);

    assert!(put_object(
        &client,
        &bucket,
        "test/rem.typ".to_string(),
        "test".to_string()
    )
    .await
    .is_ok());

    let res = delete_object(&client, &bucket, "test/rem.typ".to_string()).await;

    assert!(res.is_ok());
}
