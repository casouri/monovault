use clap::{Arg, Command};
use fuser::{self, MountOption};
use monovault::{
    caching_remote::CachingVault, fuse::FS, local_vault::LocalVault, remote_vault::RemoteVault,
    types::*, vault_server::run_server,
};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    env_logger::init();

    let matches = Command::new("monovault")
        .version("0.1.0")
        .about("Distributed network FS")
        .arg(
            Arg::new("config")
                .short('c')
                .takes_value(true)
                .help("configuration file path")
                .required(true),
        )
        .get_matches();

    let config_path = matches.value_of("config").unwrap();
    let config_file_content =
        &fs::read_to_string(config_path).expect("Cannot read the configuration file");
    let config: Config =
        serde_json::from_str(config_file_content).expect("Cannot parse the configuration file");

    // TODO: Check for duplicate vault name.

    // Make sure mount point exists.
    let mount_point = Path::new(&config.mount_point);
    if !mount_point.exists() {
        panic!("Mount point doesn't exist");
    }

    // Make sure db_path exists.
    let db_path = Path::new(&config.db_path);
    if !db_path.exists() {
        fs::create_dir(&db_path).expect("Cannot create directory for database");
    }

    // Create local vault.
    let mut vaults: Vec<VaultRef> = vec![];
    let local_vault = LocalVault::new(&config.local_vault_name, &db_path)
        .expect("Cannot create local vault instance");
    let local_vault_arc: VaultRef = Arc::new(Mutex::new(Box::new(local_vault)));

    vaults.push(Arc::clone(&local_vault_arc));

    // Create remote vaults.
    for (peer_name, peer_address) in config.peers {
        let remote_vault = RemoteVault::new(&peer_address, &peer_name)
            .expect("Cannot create remote vault instance");
        if config.caching {
            let caching_remote = CachingVault::new(
                Arc::new(Mutex::new(Box::new(remote_vault))),
                &Path::new(&config.db_path),
                config.allow_disconnected_delete,
                config.allow_disconnected_create,
            )
            .expect("Cannot create caching remote instance");
            vaults.push(Arc::new(Mutex::new(Box::new(caching_remote))));
        } else {
            vaults.push(Arc::new(Mutex::new(Box::new(remote_vault))));
        }
    }

    // Run vault server.
    // TODO: Add restart?
    if config.share_local_vault {
        let addr = config.my_address.clone();
        let vault_ref = Arc::clone(&local_vault_arc);
        let _server_handle = thread::spawn(move || run_server(&addr, vault_ref));
    }

    // Configure and start FS.
    let mount_point_name = Path::new(&config.mount_point)
        .file_name()
        .unwrap()
        .to_string_lossy();
    let options = vec![
        MountOption::FSName(mount_point_name.clone().into_owned()),
        MountOption::CUSTOM(format!("volname={}", mount_point_name)),
        // Auto unmount on process exit (doesn't seem to work).
        MountOption::AutoUnmount,
        // Allow root user to access this file system.
        MountOption::AllowRoot,
        // Disable special character and block devices
        MountOption::NoDev,
        // Prevents Apple from generating ._ files.
        MountOption::CUSTOM("noapplexattr".to_string()),
        MountOption::CUSTOM("noappledouble".to_string()),
    ];
    let fs = FS::new(vaults);
    fuser::mount2(fs, &config.mount_point, &options).expect("Error running the file system");
}
