use std::{
    collections::HashMap,
    env,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use dotenvy::dotenv;
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use minio::s3::{creds::StaticProvider, types::S3Api, ClientBuilder};
use operational_transform::OperationSeq;
use redis::{
    aio::{ConnectionManager, ConnectionManagerConfig},
    AsyncCommands, Client,
};
use serde::Deserialize;
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{
    filters::ws::{Message, WebSocket},
    Filter,
};

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);

type Users = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct RawOperation {
    text: String,
    range_offset: u64,
    range_length: u64,
}

#[allow(clippy::identity_op)]
const EXP_TIME: i64 = 1 * 60 * 1; // 30 minutes

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

    let users = Users::default();

    let users = warp::any().map(move || users.clone());

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

    let routes = warp::path("users")
        // The `ws()` filter will prepare the Websocket handshake.
        .and(warp::ws())
        .and(users)
        .and(warp_pool)
        .and(warp_minio)
        .map(|ws: warp::ws::Ws, users, pool, minio| {
            // And then our closure will be called when it completes...
            ws.on_upgrade(move |socket| user_connected(socket, users, pool, minio))
        });

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}

async fn user_connected(
    ws: WebSocket,
    users: Users,
    mut pool: ConnectionManager,
    minio: minio::s3::Client,
) {
    let my_id = NEXT_USER_ID.fetch_add(1, Ordering::Relaxed);
    eprintln!("new user: {}", my_id);

    let (mut user_ws_tx, mut user_ws_rx) = ws.split();
    let (tx, rx) = mpsc::unbounded_channel();
    let mut rx = UnboundedReceiverStream::new(rx);

    tokio::task::spawn(async move {
        while let Some(message) = rx.next().await {
            user_ws_tx
                .send(message)
                .unwrap_or_else(|e| {
                    eprintln!("websocket send error: {}", e);
                })
                .await;
        }
    });

    // Save the sender in our list of users.
    users.write().await.insert(my_id, tx.clone());

    let mut id = String::new();
    // let mut a = OperationSeq::default();
    let mut s: Option<String> = None;

    while let Some(result) = user_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("websocket error(uid={}): {}", my_id, e);
                break;
            }
        };

        if msg.is_text() || msg.is_binary() {
            let text = msg.to_str().unwrap();
            if text.starts_with("EDIT ") {
                if id.is_empty() {
                    continue;
                }
                let raw_seq =
                    serde_json::from_str::<Vec<RawOperation>>(text.replace("EDIT ", "").as_str())
                        .unwrap();
                let mut b = OperationSeq::default();
                b.retain(raw_seq.first().unwrap().range_offset);
                for raw in raw_seq {
                    // b.retain((raw.range.start_column - 1) as u64);
                    if raw.text.is_empty() {
                        b.delete(raw.range_length);
                    } else {
                        b.insert(raw.text.as_str());
                        if raw.range_length > 0 {
                            b.delete(raw.range_length);
                        }
                    }
                }

                if s.is_some() {
                    let tmp = s.clone().unwrap();
                    if b.base_len() < tmp.len() {
                        b.retain((tmp.len() - b.base_len()).try_into().unwrap());
                    }
                    match b.apply(tmp.as_str()) {
                        // cant apply two deletes at a time
                        Ok(value) => {
                            s = Some(value);
                            let _: () = pool.set(&id, s.clone().unwrap()).await.unwrap();
                            let _: () = pool.expire(&id, EXP_TIME).await.unwrap();
                            let phantom_key = format!("phantom_{}", &id);
                            let _: () = pool.expire(&phantom_key, EXP_TIME - 3).await.unwrap();
                            println!("b: {:#?}, s: {}, s len: {}", b, tmp, tmp.len());
                        }
                        Err(e) => {
                            println!(
                                "error: {}, b: {:#?}, s: {}, s len: {}",
                                e,
                                b,
                                tmp,
                                tmp.len()
                            );
                        }
                    }
                }
                println!("value: {}", s.clone().unwrap());
            } else if text.starts_with("INIT ") {
                id = text.replace("INIT ", "");
                let value: String = {
                    let ret = pool.get(&id).await;
                    if let Ok(value) = ret {
                        value
                    } else {
                        let res = minio
                            .get_object(
                                &env::var("MINIO_BUCKET").unwrap(),
                                format!(
                                    "users/cRI1h7gH4gjzAIB1zl2V/projects/{}/files/main.typ",
                                    &id
                                )
                                .as_str(),
                            )
                            .send()
                            .await
                            .unwrap();
                        let bytes = res.content.to_segmented_bytes().await.unwrap().to_bytes();
                        match String::from_utf8(bytes.to_vec()) {
                            Ok(value) => value,
                            Err(e) => {
                                eprintln!("error: {}", e);
                                "".to_string()
                            }
                        }
                    }
                };

                println!("value: {}", value);
                s = Some(value);
                let phantom_key = format!("phantom_{}", &id);
                let _: () = pool.set(&phantom_key, "").await.unwrap();
                let _: () = pool.expire(&phantom_key, EXP_TIME - 3).await.unwrap();
                tx.send(Message::text(format!("INIT {}", s.clone().unwrap())))
                    .unwrap();
            } else {
                println!("text: {}", text);
            }
        }
    }

    user_disconnected(my_id, &users).await;
}

async fn user_disconnected(my_id: usize, users: &Users) {
    eprintln!("good bye user: {}", my_id);

    // Stream closed up, so remove from the user list
    users.write().await.remove(&my_id);
}
