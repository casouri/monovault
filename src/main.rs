use clap::{Arg, Command};
use fuser::{self, MountOption};
use monovault::{database::Database, fuse::FS, local_vault::LocalVault, types::*};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

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

    let local_vault = LocalVault::new(&config.local_vault_name, &db_path)
        .expect("Cannot create local vault instance");

    let options = vec![
        MountOption::FSName("monovault".to_string()),
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
    let fs = FS::new(vec![Box::new(local_vault)]);
    fuser::mount2(fs, &config.mount_point, &options).unwrap();
}
