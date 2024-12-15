use std::env;

use dotenvy::dotenv;
use minio::s3::{creds::StaticProvider, error::Error, Client, ClientBuilder};
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
