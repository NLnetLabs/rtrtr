use std::env::current_dir;
use std::process::exit;
use clap::{App, crate_authors, crate_version};
use futures::future::pending;
use log::error;
use tokio::runtime;
use rtrtr::config::Config;
use rtrtr::log::ExitError;
use rtrtr::manager::Manager;


fn _main() -> Result<(), ExitError> {
    Config::init()?;
    let matches = Config::config_args(
        App::new("rtrtr")
        .version(crate_version!())
        .author(crate_authors!())
        .about("collecting, processing and distributing route filtering data")
    ).get_matches();
    let cur_dir = match current_dir() {
        Ok(dir) => dir,
        Err(err) => {
            error!(
                "Fatal: cannot get current directory ({}). Aborting.",
                err
            );
            return Err(ExitError);
        }
    };
    let mut manager = Manager::new();
    let mut config = Config::from_arg_matches(
        &matches, &cur_dir, &mut manager
    )?;
    let mut runtime = runtime::Builder::new()
        .threaded_scheduler()
        .enable_all()
        .build()
        .unwrap();
    config.http.run(manager.metrics(), &runtime)?;
    manager.spawn(&mut config, &runtime);
    runtime.block_on(pending())
}

fn main() {
    match _main() {
        Ok(_) => exit(0),
        Err(ExitError) => exit(1),
    }
}

