use crate::types::*;
use log::{debug, info};
use rusqlite::params;
use std::path::{Path, PathBuf};

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
}

impl Database {
    /// The database file is created at `db_path/store.sqlite3`.
    pub fn new(db_path: &Path) -> VaultResult<Database> {
        let connection = rusqlite::Connection::open(&db_path.join("store.sqlite3"))?;
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
primary key (file)
);",
            [],
        )?;
        Ok(Database {
            db: connection,
            db_path: db_path.to_path_buf(),
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

    /// Return attributes of `file`.
    pub fn attr(&mut self, file: Inode) -> VaultResult<DirEntry> {
        // Root directory is never recorded in the Type table. It is
        // annoying to insert it if not exists, so we just special
        // case it.
        if file == 1 {
            return Ok(DirEntry {
                inode: 1,
                name: "/".to_string(),
                kind: VaultFileType::Directory,
            });
        }
        let entry =
            self.db
                .query_row("select name, type from Type where file=?", [file], |row| {
                    Ok(DirEntry {
                        inode: file,
                        name: row.get_unwrap(0),
                        kind: {
                            if row.get_unwrap::<_, i32>(1) == 0 {
                                VaultFileType::File
                            } else {
                                VaultFileType::Directory
                            }
                        },
                    })
                })?;
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
            "insert into Type (file, name, type) values (?, ?, ?)",
            params![child, name.to_string(), type_val],
        )?;
        transaction.execute(
            "insert into HasChild (parent, child) values (?, ?)",
            [parent, child],
        )?;
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
                let grandchildren = self.readdir(child)?;
                let mut empty = true;
                for gchild in grandchildren {
                    if gchild.name != "." && gchild.name != ".." {
                        empty = false;
                    }
                }
                if !empty {
                    return Err(VaultError::DirectoryNotEmpty(child));
                }
            }
            VaultFileType::File => (),
        }

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

    /// List directory entries of `file`. The listing includes "." and
    /// "..", but if `file` is vault root, ".." is not included.
    pub fn readdir(&mut self, file: Inode) -> VaultResult<Vec<DirEntry>> {
        let mut result = vec![];
        result.push(DirEntry {
            inode: file,
            name: ".".to_string(),
            kind: VaultFileType::Directory,
        });
        // If this is the root of this vault, we let fuse to add
        // parent directory.
        if file != 1 {
            let parent =
                self.db
                    .query_row("select parent from HasChild where child=?", [file], |row| {
                        Ok(row.get_unwrap(0))
                    })?;
            result.push(DirEntry {
                inode: parent,
                name: "..".to_string(),
                kind: VaultFileType::Directory,
            });
        }
        // Get each entry from the database.
        let children = {
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
        // Self.attr accesses database too, so it can't be interleaved
        // with quering.
        for child in children {
            let entry = self.attr(child)?;
            result.push(entry);
        }
        Ok(result)
    }
}
