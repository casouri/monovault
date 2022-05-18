use crate::types::*;
use rusqlite::OptionalExtension;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

// ROOT of the data base is basically /db, where / is the mounting
// point of the file system. I would hash paths and store data under
// hashed names. Eg, hash /a/b to 1220338343, and store the content in
// /db/data/1220338343. This way we avoid messing with directories.
// Set and get basically gives you a key-value store that local_vault
// can use for storing meta data, like directories and anything else.
// The simplest implementation is just serialize the value to string,
// and store as a file. Eg, set("/a/b", stuff) -> hash /a/b to
// 1220338343, and store serialized stuff under /db/kv/1220338343.

/// Database provides object storage and key-value storage.
#[derive(Debug)]
pub struct Database {
    root: PathBuf,
    hasher: DefaultHasher,
    db: rusqlite::Connection,
}

impl Database {
    pub fn new(root: &Path) -> VaultResult<Database> {
        let db_path = root.join("store.sqlite3");
        let connection = rusqlite::Connection::open(&db_path)?;
        connection.execute(
            "create table if not exists Store (
key char(512) primary key,
value char(128)
);",
            [],
        )?;
        Ok(Database {
            root: root.to_path_buf(),
            hasher: DefaultHasher::new(),
            db: connection,
        })
    }

    fn real_path(&mut self, path: &Path) -> PathBuf {
        let hasher = &mut self.hasher;
        path.to_string_lossy().into_owned().hash(hasher);
        let name = hasher.finish().to_string();
        let real_path = self.root.join(name).to_path_buf();
        real_path
    }

    pub fn open(&mut self, path: &Path, option: OpenOptions) -> VaultResult<File> {
        let real_path = self.real_path(path);
        let file = option.open(real_path)?;
        Ok(file)
    }

    pub fn file_exists(&mut self, path: &Path) -> VaultResult<bool> {
        let real_path = self.real_path(path);
        let file_exists = real_path.exists();
        Ok(file_exists)
    }

    fn key_exists(&mut self, path: &Path) -> VaultResult<bool> {
        let path_str = path.to_string_lossy().into_owned();
        let result: Option<String> = self
            .db
            .query_row("select value from Store where key = ?", [path_str], |row| {
                row.get(0)
            })
            .optional()?;
        match result {
            None => Ok(false),
            Some(data) => Ok(true),
        }
    }

    pub fn delete(&mut self, path: &Path) -> VaultResult<()> {
        let real_path = self.real_path(path);
        if !real_path.exists() {
            return Err(VaultError::FileNotExist(
                path.to_string_lossy().into_owned(),
            ));
        }
        Ok(std::fs::remove_file(real_path)?)
    }

    // Set and get are used for storing metadata, like directory,
    // version, etc. Setting None means delete the value.
    pub fn set<T: Serialize>(&mut self, path: &Path, value: Option<T>) -> VaultResult<()> {
        let transaction = self.db.transaction()?;
        transaction.execute("delete from Store where key=?", [path.to_str()])?;
        if let Some(val) = value {
            let val_str = serde_json::to_string(&val)?;
            let path_str = path.to_string_lossy().into_owned();
            transaction.execute(
                "insert into Store (key, value) values (?, ?)",
                [path_str, val_str],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get<T: DeserializeOwned>(&mut self, path: &Path) -> VaultResult<Option<T>> {
        let path_str = path.to_string_lossy().into_owned();
        let result: Option<String> = self
            .db
            .query_row("select value from Store where key = ?", [path_str], |row| {
                row.get(0)
            })
            .optional()?;
        match result {
            None => Ok(None),
            Some(data) => Ok(Some(serde_json::from_str(&data)?)),
        }
    }
}
