use std::process::exit;
use clap::{Command, crate_authors, crate_version};
use daemonbase::error::ExitError;
use daemonbase::logging::Logger;
use futures::future::pending;
use tokio::runtime;
use rtrtr::config::Config;


fn _main() -> Result<(), ExitError> {
    Logger::init_logging()?;
    let matches = Config::config_args(
        Command::new("rtrtr")
        .version(crate_version!())
        .author(crate_authors!())
        .about("collecting, processing and distributing route filtering data")
    ).get_matches();
    let (mut manager, mut config) = Config::from_arg_matches(&matches)?;
    Logger::from_config(&config.log)?.switch_logging(false)?;
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = runtime.handle();
    config.http.run(manager.metrics(), manager.http_resources(), &runtime)?;
    manager.spawn(&mut config.units, &mut config.targets, handle);
    runtime.block_on(pending())
}

fn main() {
    match _main() {
        Ok(_) => exit(0),
        Err(err) => err.exit(),
    }
}

