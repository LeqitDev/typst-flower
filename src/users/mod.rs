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
use operational_transform::OperationSeq;
use structs::{ClientRequest, Document, Entry, Revision, ServerResponse};
use tokio::{
    sync::{mpsc, RwLock},
    time::Instant,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::filters::ws::{Message, WebSocket};

pub mod bucket;
pub mod structs;

fn get_project_path(user_id: &String, project_id: &String) -> String {
    format!("users/{}/projects/{}/files", user_id, project_id)
}

fn send_response(tx: &mpsc::UnboundedSender<Message>, response: ServerResponse) {
    println!("sending response: {:?}", response);
    tx.send(Message::text(serde_json::to_string(&response).unwrap()))
        .unwrap();
}

pub struct Project {
    pub state: RwLock<HashMap<String, Arc<Document>>>,
    pub users: RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>,
    pub id_count: AtomicUsize,
    pub last_access: RwLock<Instant>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            state: RwLock::new(HashMap::new()),
            users: RwLock::new(HashMap::new()),
            id_count: AtomicUsize::new(1),
            last_access: RwLock::new(Instant::now()),
        }
    }
}

impl Project {
    async fn get_file_entry(
        &self,
        path: &String,
        minio: &minio::s3::Client,
        bucket: &str,
    ) -> Arc<Document> {
        let binding = self.state.read().await;
        let has_doc = binding.get(path);
        if let Some(entry) = has_doc {
            entry.clone()
        } else {
            drop(binding);
            let content = get_object(minio, bucket, path).await.unwrap();

            let doc = Arc::new(Document::new(content));
            self.state.write().await.insert(path.clone(), doc.clone());

            doc
        }
    }

    pub async fn user_connected(
        &self,
        ws: WebSocket,
        project_id: String,
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
                println!("received message: ''{:?}''", msg);
                let request = ClientRequest::from(msg);

                match request.action {
                    structs::ActionType::Init => {
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
                                let mut entrys = Vec::new();

                                for file in project_files {
                                    let entry =
                                        self.get_file_entry(&file.name, &minio, &bucket).await;
                                    entrys.push(Entry::new(
                                        file.name,
                                        entry.state.read().await.text.clone(),
                                        structs::Revision::Some(
                                            entry.state.read().await.operations.len() as u64,
                                        ),
                                    ));
                                }

                                send_response(
                                    &tx,
                                    ServerResponse::new(
                                        structs::Revision::None,
                                        structs::PayloadType::init_ok(entrys),
                                    ),
                                );
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
                    } => todo!(), // notify all users about the new file
                    structs::ActionType::DeleteFile { path } => todo!(),
                    structs::ActionType::RenameFile { old_path, new_path } => todo!(),
                    structs::ActionType::CreateDirectory { path } => todo!(),
                    structs::ActionType::DeleteDirectory { path } => todo!(),
                    structs::ActionType::EditFile { path, changes } => {
                        let entry = self.get_file_entry(&path, &minio, &bucket).await;
                        let user_op = changes
                            .clone()
                            .into_user_op(request.client_id, request.revision);

                        match entry.apply_operation(user_op).await {
                            Ok(_) => {
                                send_response(
                                    &tx,
                                    ServerResponse::new(
                                        Revision::Some(
                                            entry.state.read().await.operations.len() as u64
                                        ),
                                        structs::PayloadType::edit_file_ok(path),
                                    ),
                                );
                            }
                            Err(e) => {
                                send_response(
                                    &tx,
                                    ServerResponse::new(
                                        structs::Revision::None,
                                        structs::PayloadType::Error {
                                            message: e.to_string(),
                                        },
                                    ),
                                );
                            }
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
                                    structs::Revision::Some(
                                        entry.state.read().await.operations.len() as u64,
                                    ),
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
