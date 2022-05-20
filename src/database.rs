use crate::types::*;
use rusqlite::params;
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
            "create table if not exists HasChild (
parent int,
child int,
primary key (parent, child)
);",
            [],
        )?;
        connection.execute(
            "create table if not exists Type (
file int,
name char(100),
type int,
primary key (dir)
);",
            [],
        )?;
        connection.execute(
            "create table if not exists Inode (
only int
inode int
primary key (only)
);",
            [],
        )?;
        Ok(Database {
            root: root.to_path_buf(),
            hasher: DefaultHasher::new(),
            db: connection,
        })
    }

    pub fn path(&self) -> PathBuf {
        self.root.clone()
    }

    pub fn largest_inode(&mut self) -> Inode {
        match self
            .db
            .query_row("select inode from Type order by inode desc", [], |row| {
                Ok(row.get(0)?)
            }) {
            Ok(inode) => inode,
            _ => 1024,
        }
    }

    /// We don' check for duplicate, etc for now.
    pub fn add_file(
        &mut self,
        parent: Inode,
        child: Inode,
        name: &str,
        kind: VaultFileType,
    ) -> VaultResult<()> {
        let transaction = self.db.transaction()?;
        let type_val = match kind {
            VaultFileType::File => 0,
            VaultFileType::Directory => 1,
        };
        transaction.execute(
            "insert into Type (file, name, type) vaules (?, ?, ?)",
            params![child, name.to_string(), type_val],
        )?;
        transaction.execute(
            "insert into HasChild (parent, child) vaules (?, ?)",
            [parent, child],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// We don't check for consistency for now.
    pub fn remove_file(&mut self, child: Inode) -> VaultResult<()> {
        let parent = self.db.query_row(
            "select parent from HasChild where child=?",
            [child],
            |row| Ok(row.get(0)?),
        )?;
        let transaction = self.db.transaction()?;
        transaction.execute(
            "delete from HasChild where parent=? and child=?",
            [parent, child],
        )?;
        transaction.execute("delete from Type where file=?", [child])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn readdir(&mut self, file: Inode) -> VaultResult<Vec<DirEntry>> {
        let mut statment = self
            .db
            .prepare("select child from HasChild where parent=?")?;
        let children = statment.query_map([file], |row| Ok(row.get(0)?))?;
        let mut result = vec![];
        for child in children {
            let child: u64 = child.unwrap();
            let entry =
                self.db
                    .query_row("select name, type from Type where file=?", [child], |row| {
                        Ok(DirEntry {
                            inode: child,
                            name: row.get(0).unwrap(),
                            kind: match row.get::<_, i32>(1) {
                                Ok(ty) => {
                                    if ty == 0 {
                                        Ok(VaultFileType::File)
                                    } else {
                                        Ok(VaultFileType::Directory)
                                    }
                                }
                                Err(err) => Err(err),
                            }?,
                        })
                    })?;
            result.push(entry);
        }
        Ok(result)
    }
}
