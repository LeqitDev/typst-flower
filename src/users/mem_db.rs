use std::time::SystemTime;

use operational_transform::OperationSeq;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};

use super::{
    mem_redis::{get_entry, get_phantom_token, has_entry, set_entry, set_entry_timeout},
    structs::{Revision, EXP_TIME},
};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CachedFile {
    pub content: String,
    pub operations: Vec<OperationSeq>,
}

impl CachedFile {
    pub fn init(content: String) -> Self {
        Self {
            content,
            operations: vec![],
        }
    }

    pub fn with_operations(content: String, operations: Vec<OperationSeq>) -> Self {
        Self {
            content,
            operations,
        }
    }

    pub async fn from_db(path: &String, pool: &mut ConnectionManager) -> Option<Self> {
        if has_entry(pool, path).await.unwrap() {
            let value = get_entry(pool, path).await.unwrap();

            Some(serde_json::from_str(&value).unwrap())
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct RedisEntry {
    pub id: String,
    pub value: CachedFile,
}

impl RedisEntry {
    pub fn new(id: String, value: CachedFile) -> Self {
        Self { id, value }
    }

    pub async fn insert(&self, pool: &mut ConnectionManager) {
        set_entry(pool, &self.id, serde_json::to_string(&self.value).unwrap())
            .await
            .unwrap();
        set_entry_timeout(pool, &self.id, EXP_TIME).await.unwrap();

        set_entry(pool, &get_phantom_token(&self.id), "".to_string())
            .await
            .unwrap();
        set_entry_timeout(pool, &get_phantom_token(&self.id), EXP_TIME - 3)
            .await
            .unwrap();
    }
}
