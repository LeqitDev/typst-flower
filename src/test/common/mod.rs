use std::env;

use dotenvy::dotenv;
use minio::s3::{creds::StaticProvider, error::Error, Client, ClientBuilder};
use redis::{
    aio::{ConnectionManager, ConnectionManagerConfig},
    RedisError,
};
use tokio::sync::mpsc;

pub async fn bucket_setup() -> Result<Client, Error> {
    // pretty_env_logger::init();

    dotenv().unwrap();

    let host_env = env::var("MINIO_ENDPOINT").unwrap();
    let port_env = env::var("MINIO_PORT").unwrap();
    let access_key = env::var("MINIO_USER").unwrap();
    let secret_key = env::var("MINIO_PW").unwrap();

    let static_provider = StaticProvider::new(&access_key, &secret_key, None);

    ClientBuilder::new(format!("{}:{}", host_env, port_env).parse().unwrap())
        .provider(Some(Box::new(static_provider)))
        .build()
}

pub async fn redis_setup() -> Result<ConnectionManager, RedisError> {
    let redis_client = redis::Client::open("redis://127.0.0.1:6379/?protocol=resp3").unwrap();
    let (tx, _) = mpsc::unbounded_channel();
    let config = ConnectionManagerConfig::new().set_push_sender(tx);
    ConnectionManager::new_with_config(redis_client, config).await
}
