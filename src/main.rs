use std::process::exit;
use clap::{Command, crate_authors, crate_version};
use daemonbase::error::ExitError;
use daemonbase::logging::Logger;
use futures::future::pending;
use tokio::runtime;
use rtrtr::config::Config;
use rtrtr::manager::Manager;


fn _main() -> Result<(), ExitError> {
    Logger::init_logging()?;
    let matches = Config::config_args(
        Command::new("rtrtr")
        .version(crate_version!())
        .author(crate_authors!())
        .about("collecting, processing and distributing route filtering data")
    ).get_matches();
    let mut manager = Manager::new();
    let mut config = Config::from_arg_matches(
        &matches, &mut manager
    )?;
    Logger::from_config(&config.log)?.switch_logging(false)?;
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    config.http.run(manager.metrics(), manager.http_resources(), &runtime)?;
    manager.spawn(&mut config, &runtime);
    runtime.block_on(pending())
}

fn main() {
    match _main() {
        Ok(_) => exit(0),
        Err(err) => err.exit(),
    }
}

