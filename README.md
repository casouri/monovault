# Setup environment

If you are on a Mac, install FUSE from https://osxfuse.github.io. Then
do the following:

```
sudo ln -s /usr/local/lib/pkgconfig/fuse.pc /usr/local/lib/pkgconfig/osxfuse.pc
export PKG_CONFIG_PATH="/usr/local/lib/pkgconfig"
```

Now you can run `cargo check` and it won’t complain not being able to
find osxfuse.pc.
