use clap::{App, Arg, crate_authors, crate_version};
use futures::future::pending;
use tokio::runtime;
use rtrtr::config::{Config, ConfigError, ConfigFile};
use rtrtr::manager::Manager;


fn main() {
    let matches = App::new("rtrtr")
        .version(crate_version!())
        .author(crate_authors!())
        .about("The RPKI Data Express Mail Service")
        .arg(Arg::with_name("config")
            .short("c")
            .long("config")
            .value_name("PATH")
            .takes_value(true)
            .required(true)
            .help("path to the config file")
        )
        .get_matches();

    simple_logging::log_to_stderr(log::LevelFilter::Debug);

    let conf_path = matches.value_of("config").unwrap();
    let conf = match ConfigFile::load(&conf_path) {
        Ok(conf) => conf,
        Err(err) => {
            eprintln!("Failed to read config file '{}': {}", conf_path, err);
            return
        }
    };

    let mut manager = Manager::default();
    let res = manager.load(|| {
        match Config::from_toml(conf.bytes()) {
            Ok(conf) => {
                let (units, targets, params) = conf.into_parts();
                Ok((units, (targets, params)))
            }
            Err(err) => Err(ConfigError::new(err, &conf)),
        }
    });

    let (targets, _params) = match res {
        Ok(some) => some,
        Err(errs) => {
            eprintln!("Found {} error(s) in config file:", errs.len());
            for err in errs.iter() {
                eprintln!("{}", err);
            }
            return;
        }
    };

    let mut runtime = runtime::Builder::new()
        .threaded_scheduler()
        .enable_all()
        .build()
        .unwrap();

    manager.spawn_units(&runtime);
    targets.spawn_all(&runtime);

    runtime.block_on(pending())
}

