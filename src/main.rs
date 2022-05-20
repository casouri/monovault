use clap::{Arg, Command};
use fuse;
use monovault::{database::Database, fuse::FS, local_vault::LocalVault, types::*};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn main() {
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

    let mount_point = Path::new(&config.mount_point);
    if !mount_point.exists() {
        panic!("Mount point doesn't exist");
    }

    let db_path = mount_point.join("db");
    if !db_path.exists() {
        fs::create_dir(&db_path).expect("Cannot create directory for database");
    }

    let database = Arc::new(Mutex::new(
        Database::new(&db_path).expect("Cannot create database"),
    ));
    let local_vault = LocalVault::new(&config.local_vault_name, Arc::clone(&database))
        .expect("Cannot create local vault instance");

    let fs = FS::new(config.clone(), vec![Box::new(local_vault)]);
    fuse::mount(fs, &config.mount_point, &[]).unwrap();
}
