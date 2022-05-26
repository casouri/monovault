use clap::{Arg, Command};
use fuser::{self, MountOption};
use monovault::{
    fuse::FS, local_vault::LocalVault, remote_vault::RemoteVault, types::*,
    vault_server::run_server,
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
    let config: Config = serde_json::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();

    // TODO: Check for duplicate vault name.

    let mount_point = Path::new(&config.mount_point);
    if !mount_point.exists() {
        panic!("Mount point doesn't exist");
    }

    let db_path = Path::new(&config.db_path);
    if !db_path.exists() {
        fs::create_dir(&db_path).expect("Cannot create directory for database");
    }

    // Create vault instances.
    let mut vaults: Vec<Arc<Mutex<Box<dyn Vault>>>> = vec![];
    let local_vault: Arc<Mutex<Box<dyn Vault>>> = Arc::new(Mutex::new(Box::new(
        LocalVault::new(&config.local_vault_name, &db_path)
            .expect("Cannot create local vault instance"),
    )));
    vaults.push(Arc::clone(&local_vault));

    for (peer_name, peer_address) in config.peers {
        let remote_vault = RemoteVault::new(&peer_address, &peer_name)
            .expect("Cannot create remote vault instance");
        vaults.push(Arc::new(Mutex::new(Box::new(remote_vault))));
    }

    // Run vault server.
    // FIXME: Add panic restart and error report.
    let addr = config.my_address.clone();
    let vault_ref = Arc::clone(&local_vault);
    let server_handle = thread::spawn(move || run_server(&addr, vault_ref));
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
    fuser::mount2(fs, &config.mount_point, &options).unwrap();
}
