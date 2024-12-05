use futures_util::StreamExt;
use minio::s3::{
    builders::ObjectContent,
    error::Error,
    response::RemoveObjectResponse,
    types::{ListEntry, S3Api, ToStream},
    Client,
};

// get object
pub async fn get_object(minio: &Client, bucket: &str, path: &str) -> Result<String, Error> {
    match minio.get_object(bucket, path).send().await {
        Ok(res) => {
            let seg_bytes = res.content.to_segmented_bytes().await?.to_bytes();

            Ok(String::from_utf8(seg_bytes.to_vec()).unwrap())
        }
        Err(e) => Err(e),
    }
}

// put object
pub async fn put_object(
    minio: &Client,
    bucket: &str,
    path: String,
    data: String,
) -> Result<(), Error> {
    match minio
        .put_object(
            bucket,
            &path,
            ObjectContent::from(data).to_segmented_bytes().await?,
        )
        .send()
        .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

// get object list (dir)
pub async fn get_object_list(
    minio: &Client,
    bucket: &str,
    dir: String,
) -> Result<Vec<ListEntry>, Error> {
    let mut res = minio
        .list_objects(bucket)
        .prefix(Some(dir))
        .recursive(true)
        .to_stream()
        .await;

    if let Some(result) = res.next().await {
        match result {
            Ok(objects) => Ok(objects.contents),
            Err(e) => Err(e),
        }
    } else {
        Ok(vec![])
    }
}

// delete object
pub async fn delete_object(
    minio: &Client,
    bucket: &str,
    path: String,
) -> Result<RemoveObjectResponse, Error> {
    minio.remove_object(bucket, path.as_str()).send().await
}
