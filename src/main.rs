use clap::{App, crate_authors, crate_version};
use rtrtr::{Config, Server, ExitError};

async fn run() -> Result<(), ExitError> {
    let matches = Config::config_args(Config::config_args(
        App::new("rtrtr")
            .version(crate_version!())
            .author(crate_authors!())
            .about("The RPKI Data Express Mail Service")
    )).get_matches();
    Server::new(Config::from_arg_matches(&matches)?).run().await
}

#[tokio::main]
async fn main() {
    match run().await {
        Ok(()) => { }
        Err(err) => err.exit()
    }
}

