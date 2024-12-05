use std::{env, sync::Arc};

use dotenvy::dotenv;
use minio::s3::{creds::StaticProvider, ClientBuilder};
use redis::{
    aio::{ConnectionManager, ConnectionManagerConfig},
    AsyncCommands, Client,
};
use tokio::sync::mpsc;
use users::{structs::ServerState, Project};
use warp::{reject::Rejection, reply::Reply, Filter};

mod users;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    dotenv().unwrap();

    let host_env = env::var("MINIO_ENDPOINT").unwrap();
    let port_env = env::var("MINIO_PORT").unwrap();
    let access_key = env::var("MINIO_USER").unwrap();
    let secret_key = env::var("MINIO_PW").unwrap();

    let static_provider = StaticProvider::new(&access_key, &secret_key, None);

    let minio_client = ClientBuilder::new(format!("{}:{}", host_env, port_env).parse().unwrap())
        .provider(Some(Box::new(static_provider)))
        .build()
        .unwrap();

    let state = ServerState::default();

    let state = warp::any().map(move || state.clone());

    let redis_client = Client::open("redis://127.0.0.1:6379/?protocol=resp3").unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = ConnectionManagerConfig::new().set_push_sender(tx);
    let mut pool = ConnectionManager::new_with_config(redis_client, config)
        .await
        .unwrap();
    redis::cmd("CONFIG")
        .arg("SET")
        .arg("notify-keyspace-events")
        .arg("Ex")
        .exec_async(&mut pool)
        .await
        .unwrap();
    pool.subscribe("__keyevent@0__:expired").await.unwrap();

    let mut event_pool = pool.clone();
    let event_minio = minio_client.clone();
    tokio::task::spawn(async move {
        while let Some(message) = rx.recv().await {
            let id = match &message.data[1] {
                redis::Value::BulkString(vec) => String::from_utf8(vec.to_vec()).unwrap(),
                redis::Value::SimpleString(s) => s.to_string(),
                _ => "".to_string(),
            };
            if id.is_empty() {
                continue;
            }
            if id.starts_with("phantom_") {
                let id = id.replace("phantom_", "");
                if let Ok(ret) = event_pool.get::<String, String>(id.clone()).await {
                    event_minio
                        .put_object_content(
                            env::var("MINIO_BUCKET").unwrap().as_str(),
                            format!("users/cRI1h7gH4gjzAIB1zl2V/projects/{}/files/main.typ", &id)
                                .as_str(),
                            ret.clone(),
                        )
                        .send()
                        .await
                        .unwrap();
                    println!("message: {:?}, val: {}", message, ret);
                }
            }
        }
    });

    let warp_pool = warp::any().map(move || pool.clone());

    let warp_minio = warp::any().map(move || minio_client.clone());

    let routes = warp::path!("users" / String)
        // The `ws()` filter will prepare the Websocket handshake.
        .and(warp::ws())
        .and(state.clone())
        .and(warp_pool)
        .and(warp_minio)
        .and_then(socket_handler);

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}

async fn socket_handler(
    id: String,
    ws: warp::ws::Ws,
    state: ServerState,
    pool: ConnectionManager,
    minio: minio::s3::Client,
) -> Result<impl Reply, Rejection> {
    let project = {
        let mut projects = state.projects.lock().unwrap();
        if let Some(project) = projects.get(&id) {
            project.clone()
        } else {
            let project = Arc::new(Project::default());
            projects.insert(id.clone(), project.clone());
            project
        }
    };

    Ok(ws.on_upgrade(|socket| async move { project.user_connected(socket, pool, minio).await }))
}

#[cfg(test)]
mod test;
