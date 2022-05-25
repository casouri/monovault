use crate::types::*;
use log::{debug, info};
use rusqlite::params;
use std::path::{Path, PathBuf};
use std::time;

/// Database is used for maintaining meta information, eg, which files
/// are contained in a directory, what's the type of each file
/// (regular file or directory). The database has two tables, HasChild
/// table records parent-child relationships, Type table records file
/// name and type (file/directory).
#[derive(Debug)]
pub struct Database {
    /// The sqlite database connection.
    db: rusqlite::Connection,
    /// The path containing the database file and cache files.
    db_path: PathBuf,
    db_name: String,
}

/// Setup the database if not already set up.
fn setup_db(connection: &mut rusqlite::Connection) -> VaultResult<()> {
    // Create tables.
    connection.execute(
        "create table if not exists HasChild (
parent int,
child int,
primary key (parent, child),
foreign key (parent, child) references Type(file, file)
);",
        [],
    )?;
    connection.execute(
        "create table if not exists Type (
file int,
name char(100),
type int,
atime int,
mtime int,
version int,
primary key (file)
);",
        [],
    )?;
    // Insert root directory if not exists.
    match connection.query_row::<u64, _, _>("select file from Type where file=1", [], |row| {
        Ok(row.get_unwrap(0))
    }) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            connection.execute(
                "insert into Type (file, name, type, atime, mtime, version) values (1, '/', 1, 0, 0, 1)",
                [],
            )?;
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

impl Database {
    /// The database file is created at `db_path/store.sqlite3`.
    pub fn new(db_path: &Path, db_name: &str) -> VaultResult<Database> {
        let mut connection =
            rusqlite::Connection::open(&db_path.join(format!("{}.sqlite3", db_name)))?;
        setup_db(&mut connection)?;

        Ok(Database {
            db: connection,
            db_path: db_path.to_path_buf(),
            db_name: db_name.to_string(),
        })
    }

    /// Return the `db_path`, the directory in which the database file resides.
    pub fn path(&self) -> PathBuf {
        self.db_path.clone()
    }

    /// Return the largest inode recorded in the database.
    pub fn largest_inode(&mut self) -> Inode {
        match self.db.query_row(
            "select child from HasChild order by child desc",
            [],
            |row| Ok(row.get_unwrap(0)),
        ) {
            Ok(inode) => inode,
            _ => 1,
        }
    }

    /// Return attributes of `file`. The `size` field is a dummy value
    /// and needs to be filled.
    pub fn attr(&mut self, file: Inode) -> VaultResult<FileInfo> {
        let entry = self.db.query_row(
            "select name, type, atime, mtime, version from Type where file=?",
            [file],
            |row| {
                Ok(FileInfo {
                    inode: file,
                    name: row.get_unwrap(0),
                    kind: {
                        if row.get_unwrap::<_, i32>(1) == 0 {
                            VaultFileType::File
                        } else {
                            VaultFileType::Directory
                        }
                    },
                    atime: row.get_unwrap(2),
                    mtime: row.get_unwrap(3),
                    version: row.get_unwrap(4),
                    // Filled by LocalVault::attr().
                    size: 0,
                })
            },
        )?;
        debug!("attr({}) => {:?}", file, &entry);
        Ok(entry)
    }

    /// Add a file/directory `child` to the database under `parent`
    /// with `name`. Duplication is detected by primary key
    /// constraints. But normally we shouldn't encounter that.
    pub fn add_file(
        &mut self,
        parent: Inode,
        child: Inode,
        name: &str,
        kind: VaultFileType,
        atime: u64,
        mtime: u64,
        version: u64,
    ) -> VaultResult<()> {
        info!(
            "add_file(parent={}, child={}, name={}, kind={:?})",
            parent, child, name, kind
        );
        // We want to count bytes, so len() is correct here.
        if name.len() > 100 {
            return Err(VaultError::FileNameTooLong(name.to_string()));
        }
        let transaction = self.db.transaction()?;
        let type_val = match kind {
            VaultFileType::File => 0,
            VaultFileType::Directory => 1,
        };
        transaction.execute(
            "insert into Type (file, name, type, atime, mtime, version) values (?, ?, ?, ?, ?, ?)",
            params![child, name.to_string(), type_val, atime, mtime, version],
        )?;
        transaction.execute(
            "insert into HasChild (parent, child) values (?, ?)",
            [parent, child],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Set `file`'s attributes: `name`, `atime`, `mtime`, `version`. None means
    /// don't change.
    pub fn set_attr(
        &mut self,
        file: Inode,
        name: Option<&str>,
        atime: Option<u64>,
        mtime: Option<u64>,
        version: Option<u64>,
    ) -> VaultResult<()> {
        info!(
            "set_attr(file={}, name={:?}, atime={:?}, mtime={:?}, version={:?})",
            file, name, atime, mtime, version
        );
        let transaction = self.db.transaction()?;
        if let Some(name) = name {
            transaction.execute("update Type set name=? where file=?", params![name, file])?;
        }
        if let Some(atime) = atime {
            transaction.execute("update Type set atime=? where file=?", params![atime, file])?;
        }
        if let Some(mtime) = mtime {
            transaction.execute("update Type set mtime=? where file=?", params![mtime, file])?;
        }
        if let Some(version) = version {
            transaction.execute(
                "update Type set version=? where file=?",
                params![version, file],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Remove a file `child` from the database.
    pub fn remove_file(&mut self, child: Inode) -> VaultResult<()> {
        info!("remove_file({})", child);
        // Check for non empty directory
        let kind = self.attr(child)?.kind;
        match kind {
            VaultFileType::Directory => {
                let (_, _, grandchildren) = self.readdir(child)?;
                let empty = grandchildren.len() == 0;
                if !empty {
                    return Err(VaultError::DirectoryNotEmpty(child));
                }
            }
            VaultFileType::File => (),
        }
        // Remove parent-child relationship and file meta.
        let parent = self.db.query_row(
            "select parent from HasChild where child=?",
            [child],
            |row| Ok(row.get_unwrap(0)),
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

    /// List directory entries of `file`. Returns a 3-tuple, first
    /// element is inode for ".", second for "..", third a vector of
    /// children. If `file` is the vault root, we don't know "..", so
    /// the second element will be 0.
    pub fn readdir(&mut self, file: Inode) -> VaultResult<(Inode, Inode, Vec<Inode>)> {
        // let mut result = vec![];
        // Get each entry from the database.
        let mut children = {
            let mut statment = self
                .db
                .prepare("select child from HasChild where parent=?")?;
            let mut rows = statment.query([file])?;
            let mut children = vec![];
            while let Some(row) = rows.next()? {
                children.push(row.get_unwrap(0));
            }
            children
        };
        info!("readdir({}) => {:?}", file, children);
        let parent = if file != 1 {
            self.db
                .query_row("select parent from HasChild where child=?", [file], |row| {
                    Ok(row.get_unwrap(0))
                })?
        } else {
            0
        };
        // Self.attr accesses database too, so it can't be interleaved
        // with quering.
        // for child in children {
        //     let entry = self.attr(child)?;
        //     result.push(entry);
        // }
        Ok((file, parent, children))
    }
}
