use std::{
    borrow::{Borrow, BorrowMut},
    collections::HashMap,
    env,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use bucket::{get_object, get_object_list};
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use mem_db::{CachedFile, RedisEntry};
use operational_transform::OperationSeq;
use redis::aio::ConnectionManager;
use structs::{ClientRequest, Document, Entry, Revision, ServerResponse};
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::filters::ws::{Message, WebSocket};

pub mod bucket;
mod mem_db;
pub mod mem_redis;
pub mod structs;

fn get_project_path(user_id: &String, project_id: &String) -> String {
    format!("users/{}/projects/{}/files", user_id, project_id)
}

fn send_response(tx: &mpsc::UnboundedSender<Message>, response: ServerResponse) {
    tx.send(Message::text(serde_json::to_string(&response).unwrap()))
        .unwrap();
}

pub struct Project {
    pub state: RwLock<HashMap<String, Arc<Document>>>,
    pub users: RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>,
    pub id_count: AtomicUsize,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            state: RwLock::new(HashMap::new()),
            users: RwLock::new(HashMap::new()),
            id_count: AtomicUsize::new(1),
        }
    }
}

impl Project {
    async fn get_file_entry(
        &self,
        path: &String,
        minio: &minio::s3::Client,
        bucket: &String,
    ) -> Arc<Document> {
        let doc = if let Some(entry) = self.state.read().await.get(path) {
            entry.clone()
        } else {
            let content = get_object(minio, bucket, path).await.unwrap();

            let doc = Arc::new(Document::new(content));
            self.state.write().await.insert(path.clone(), doc.clone());

            doc
        };

        doc
    }

    pub async fn user_connected(
        &self,
        ws: WebSocket,
        mut pool: ConnectionManager,
        minio: minio::s3::Client,
    ) {
        let my_id = self.id_count.fetch_add(1, Ordering::Relaxed);
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
        self.users.write().await.insert(my_id, tx.clone());

        // let mut a = OperationSeq::default();
        let mut project_path: Option<String> = None;

        let bucket = env::var("MINIO_BUCKET").unwrap();

        while let Some(result) = user_ws_rx.next().await {
            let msg = match result {
                Ok(msg) => msg,
                Err(e) => {
                    eprintln!("websocket error(uid={}): {}", my_id, e);
                    break;
                }
            };

            if msg.is_text() || msg.is_binary() {
                let request = ClientRequest::from(msg);

                match request.action {
                    structs::ActionType::Init { project_id } => {
                        project_path = Some(get_project_path(&request.client_id, &project_id));

                        let project_files =
                            get_object_list(&minio, &bucket, project_path.clone().unwrap()).await;

                        if let Ok(project_files) = project_files {
                            if project_files.is_empty() {
                                // No files in project (TODO: thats not right here but i dont care for now, a project can be empty)
                                send_response(
                                    &tx,
                                    ServerResponse::new(
                                        structs::Revision::None,
                                        structs::PayloadType::Error {
                                            message: "This project does not exist!".to_string(),
                                        },
                                    ),
                                );
                            } else {
                                let has_main = project_files
                                    .iter()
                                    .find(|entry| entry.name.ends_with("files/main.typ"));
                                if let Some(main) = has_main {
                                    // Main file exists in root of project
                                    let main_entry =
                                        self.get_file_entry(&main.name, &minio, &bucket).await;

                                    send_response(
                                        &tx,
                                        ServerResponse::new(
                                            Revision::Some(
                                                main_entry.state.read().await.operations.len()
                                                    as u64,
                                            ),
                                            structs::PayloadType::init_ok(Entry::new(
                                                main.name.clone(),
                                                main_entry.state.read().await.text.clone(),
                                            )),
                                        ),
                                    );
                                } else {
                                    let has_lib = project_files
                                        .iter()
                                        .find(|entry| entry.name.ends_with("files/lib.typ"));
                                    if let Some(lib) = has_lib {
                                        // Lib file exists in root of project (Improvement: Check for the entrypoint in the toml)
                                        let lib_entry =
                                            self.get_file_entry(&lib.name, &minio, &bucket).await;

                                        send_response(
                                            &tx,
                                            ServerResponse::new(
                                                Revision::Some(
                                                    lib_entry.state.read().await.operations.len()
                                                        as u64,
                                                ),
                                                structs::PayloadType::init_ok(Entry::new(
                                                    lib.name.clone(),
                                                    lib_entry.state.read().await.text.clone(),
                                                )),
                                            ),
                                        );
                                    } else {
                                        let first_file = project_files.first().unwrap(); // just get the first file in project
                                        let first_entry = self
                                            .get_file_entry(&first_file.name, &minio, &bucket)
                                            .await;

                                        send_response(
                                            &tx,
                                            ServerResponse::new(
                                                Revision::Some(
                                                    first_entry.state.read().await.operations.len()
                                                        as u64,
                                                ),
                                                structs::PayloadType::init_ok(Entry::new(
                                                    first_file.name.clone(),
                                                    first_entry.state.read().await.text.clone(),
                                                )),
                                            ),
                                        );
                                    }
                                }
                            }
                        } else {
                            send_response(
                                &tx,
                                ServerResponse::new(
                                    structs::Revision::None,
                                    structs::PayloadType::Error {
                                        message: "This project does not exist!".to_string(),
                                    },
                                ),
                            );
                        }
                    }
                    structs::ActionType::CreateFile {
                        path,
                        initial_content,
                    } => todo!(),
                    structs::ActionType::DeleteFile { path } => todo!(),
                    structs::ActionType::RenameFile { old_path, new_path } => todo!(),
                    structs::ActionType::CreateDirectory { path } => todo!(),
                    structs::ActionType::DeleteDirectory { path } => todo!(),
                    structs::ActionType::EditFile { path, changes } => {
                        let entry = self.get_file_entry(&path, &minio, &bucket).await;
                        let user_op = changes
                            .clone()
                            .into_user_op(request.client_id, request.revision);

                        if entry.apply_operation(user_op).await.is_ok() {
                            send_response(
                                &tx,
                                ServerResponse::new(
                                    Revision::Some(entry.state.read().await.operations.len() as u64),
                                    structs::PayloadType::edit_file_ok(path),
                                ),
                            );
                        } else {
                            send_response(
                                &tx,
                                ServerResponse::new(
                                    structs::Revision::None,
                                    structs::PayloadType::Error {
                                        message: "Operation failed!".to_string(),
                                    },
                                ),
                            );
                        }
                    }
                    structs::ActionType::OpenFile { path } => {
                        let entry = self.get_file_entry(&path, &minio, &bucket).await;

                        send_response(
                            &tx,
                            ServerResponse::new(
                                structs::Revision::Some(
                                    entry.state.read().await.operations.len() as u64
                                ),
                                structs::PayloadType::open_file_ok(Entry::new(
                                    path,
                                    entry.state.read().await.text.clone(),
                                )),
                            ),
                        );
                    }
                }
            }
        }

        self.user_disconnected(my_id).await;
    }

    pub async fn user_disconnected(&self, my_id: usize) {
        eprintln!("good bye user: {}", my_id);

        // Stream closed up, so remove from the user list
        self.users.write().await.remove(&my_id);
    }
}
