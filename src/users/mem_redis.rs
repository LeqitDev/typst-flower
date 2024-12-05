use redis::{aio::ConnectionManager, AsyncCommands, RedisError};

// set entry
pub async fn set_entry(
    pool: &mut ConnectionManager,
    key: &String,
    value: String,
) -> Result<(), RedisError> {
    pool.set(key, value).await
}

// get entry
pub async fn get_entry(pool: &mut ConnectionManager, key: &String) -> Result<String, RedisError> {
    match pool.get(key).await {
        Ok(value) => Ok(value),
        Err(e) => Err(e),
    }
}

// has entry
pub async fn has_entry(pool: &mut ConnectionManager, key: &String) -> Result<bool, RedisError> {
    match pool.exists(key).await {
        Ok(value) => Ok(value),
        Err(e) => Err(e),
    }
}

// set entry timeout
pub async fn set_entry_timeout(
    pool: &mut ConnectionManager,
    key: &String,
    seconds: i64,
) -> Result<(), RedisError> {
    pool.expire(key, seconds).await
}

// (private) get phantom token
pub fn get_phantom_token(id: &String) -> String {
    format!("phantom_{}", id)
}

// delete entry
pub async fn delete_entry(pool: &mut ConnectionManager, key: &String) -> Result<(), RedisError> {
    pool.del(key).await
}
