use std::{
    collections::HashMap,
    ops::Add,
    sync::{atomic::AtomicUsize, Arc, Mutex},
};

use operational_transform::{OTError, OperationSeq};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use warp::filters::ws::Message;

use super::Project;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RawOperation {
    pub text: String,
    pub range_offset: u64,
    pub range_length: u64,
    pub rest_length: u64,
}

impl RawOperation {
    pub fn is_insert(&self) -> bool {
        self.range_length == 0
    }

    pub fn is_delete(&self) -> bool {
        self.range_length > 0 && self.text.is_empty()
    }

    pub fn is_replace(&self) -> bool {
        self.range_length > 0 && !self.text.is_empty()
    }
}

impl RawOperation {
    pub fn into_user_op(self, client_id: String, revision: Revision) -> UserOperation {
        UserOperation {
            revision,
            client_id,
            operation: self.into(),
        }
    }
}

impl From<RawOperation> for OperationSeq {
    fn from(val: RawOperation) -> Self {
        let mut operation_seq = OperationSeq::default();

        operation_seq.retain(val.range_offset);

        if val.is_insert() {
            operation_seq.insert(val.text.as_str());
        } else if val.is_delete() {
            operation_seq.delete(val.range_length);
        } else if val.is_replace() {
            operation_seq.delete(val.range_length);
            operation_seq.insert(val.text.as_str());
        }

        operation_seq.retain(val.rest_length);

        operation_seq
    }
}

impl Add<u64> for Revision {
    type Output = Self;

    fn add(self, other: u64) -> Self {
        match self {
            Revision::None => Revision::Some(other),
            Revision::Some(rev) => Revision::Some(rev + other),
        }
    }
}

#[allow(clippy::identity_op)]
pub const EXP_TIME: i64 = 1 * 60 * 30; // 30 minutes

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum ActionType {
    Init,
    CreateFile {
        path: String,
        initial_content: Option<String>,
    },
    DeleteFile {
        path: String,
    },
    RenameFile {
        old_path: String,
        new_path: String,
    },
    CreateDirectory {
        path: String,
    },
    DeleteDirectory {
        path: String,
    },
    EditFile {
        path: String,
        changes: RawOperation,
    },
    OpenFile {
        path: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ClientRequest {
    pub revision: Revision,
    pub client_id: String,
    pub parent_revision: u64,
    pub timestamp: u64,
    pub action: ActionType,
}

impl From<Message> for ClientRequest {
    fn from(msg: Message) -> Self {
        let text = msg.to_str().unwrap();
        serde_json::from_str(text).unwrap()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Entry {
    pub path: String,
    pub content: String,
}

impl Entry {
    pub fn new(path: String, content: String) -> Self {
        Self { path, content }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum PayloadType {
    Error { message: String },
    InitOk { files: Vec<Entry> },
    CreateFileOk { path: String },
    DeleteFileOk { path: String },
    RenameFileOk { old_path: String, new_path: String },
    CreateDirectoryOk { path: String },
    DeleteDirectoryOk { path: String },
    EditFileOk { path: String },
    OpenFileOk { file: Entry },
}

impl PayloadType {
    pub fn error(message: String) -> Self {
        PayloadType::Error { message }
    }

    pub fn init_ok(files: Vec<Entry>) -> Self {
        PayloadType::InitOk { files }
    }

    pub fn create_file_ok(path: String) -> Self {
        PayloadType::CreateFileOk { path }
    }

    pub fn delete_file_ok(path: String) -> Self {
        PayloadType::DeleteFileOk { path }
    }

    pub fn rename_file_ok(old_path: String, new_path: String) -> Self {
        PayloadType::RenameFileOk { old_path, new_path }
    }

    pub fn create_directory_ok(path: String) -> Self {
        PayloadType::CreateDirectoryOk { path }
    }

    pub fn delete_directory_ok(path: String) -> Self {
        PayloadType::DeleteDirectoryOk { path }
    }

    pub fn edit_file_ok(path: String) -> Self {
        PayloadType::EditFileOk { path }
    }

    pub fn open_file_ok(file: Entry) -> Self {
        PayloadType::OpenFileOk { file }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "number")]
pub enum Revision {
    None,
    Some(u64),
}

impl Revision {
    pub fn inner(&self) -> u64 {
        match self {
            Revision::None => 0,
            Revision::Some(rev) => *rev,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerResponse {
    pub revision: Revision,
    pub payload: PayloadType,
}

impl ServerResponse {
    pub fn new(revision: Revision, payload: PayloadType) -> Self {
        Self { revision, payload }
    }
}

pub struct UserOperation {
    pub client_id: String,
    pub revision: Revision,
    pub operation: OperationSeq,
}

pub struct DocumentState {
    pub text: String,
    pub operations: Vec<UserOperation>,
}

pub struct Document {
    pub state: RwLock<DocumentState>,
}

impl Document {
    pub fn new(text: String) -> Self {
        Self {
            state: RwLock::new(DocumentState {
                text,
                operations: vec![],
            }),
        }
    }

    pub async fn apply_operation(&self, mut operation: UserOperation) -> Result<(), String> {
        let mut new_text = self.state.read().await.text.clone();

        println!("text before: {}", new_text);

        let len = self.state.read().await.operations.len();
        if operation.revision.inner() > len as u64 {
            return Err("Invalid revision".to_string());
        }

        println!("operation rev: {:?}", operation.revision.inner());
        for op in &self.state.read().await.operations[operation.revision.inner() as usize..] {
            println!("op: {:?}", op.operation);
            operation.operation = operation
                .operation
                .transform(&op.operation)
                .map_err(|e| {
                    format!(
                        "On transform {:?}. Op: {:?}, transformed with {:?}",
                        e.to_string(),
                        operation.operation,
                        op.operation
                    )
                })?
                .0;
        }
        if operation.operation.target_len() > 100000 {
            return Err(format!(
                "Operation too long: {}",
                operation.operation.target_len()
            ));
        }

        new_text = operation.operation.apply(new_text.as_str()).map_err(|e| {
            format!(
                "On apply {:?}. Op: {:?}.",
                e.to_string(),
                operation.operation
            )
        })?;

        println!("new_text: {}", new_text);

        self.state.write().await.text = new_text;
        self.state.write().await.operations.push(operation);

        Ok(())
    }
}

#[derive(Default, Clone)]
pub struct ServerState {
    pub projects: Arc<Mutex<HashMap<String, Arc<Project>>>>,
}
