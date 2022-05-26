# Setup environment

If you are on a Mac, install FUSE from https://osxfuse.github.io. Then
do the following:

```
sudo ln -s /usr/local/lib/pkgconfig/fuse.pc /usr/local/lib/pkgconfig/osxfuse.pc
export PKG_CONFIG_PATH="/usr/local/lib/pkgconfig"
```

Now you can run `cargo check` and it won’t complain not being able to
find osxfuse.pc.

# Test the local vault

The executable takes a single argument, the configuration file. The
configuration file (a JSON file) should look something like this:

```json
{
  "my_address": "127.0.0.1",
  "peers": {},
  "mount_point": "/Users/yuan/p/cse223/monovault/mount",
  "db_path": "/Users/yuan/p/cse223/monovault/db",
  "local_vault_name": "pandora"
}
```

"my_address" and "peers" is not yet used, "mount_point" is where you
want to mount the file system. "db_path" is a directory that contains
all the cache and database, obviously it shouldn’t be under the mount
point. "local_vault_name" is just what it is, the name of the local vault.

Run the file system like this:

```shell
cargo run -- -c /path/to/config.json
```

where `config.json` is the configuration file. To enable logging, set
`RUST_LOG`. For example,

```shell
RUST_LOG="warn,monovault::fuse=info" cargo run -- -c /path/to/config.json
```

This should log all the calls made by FUSE.

To stop the file system, just manually unmount the filesystem. To
unmount (on mac):

```shell
umount -f /path/to/mount/point
```

# Test the remote vault (with no caching)

Now we run two instances of monovault locally. Instance A:

```json
{
  "my_address": "127.0.0.1:7771",
  "peers": {
    "alice": "http://127.0.0.1:7772"
  },
  "mount_point": "/Users/yuan/p/cse223/monovault/mount",
  "db_path": "/Users/yuan/p/cse223/monovault/db",
  "local_vault_name": "yuan"
}
```

Run by

```json
cargo run -- -c /path/to/config.json
```

Instance B:

```json
{
  "my_address": "127.0.0.1:7772",
  "peers": {
    "yuan": "http://127.0.0.1:7771"
  },
  "mount_point": "/Users/yuan/p/cse223/monovault/mount2",
  "db_path": "/Users/yuan/p/cse223/monovault/db2",
  "local_vault_name": "alice"
}
```

```json
cargo run -- -c /path/to/config2.json
```
