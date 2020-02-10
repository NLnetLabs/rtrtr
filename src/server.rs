
use std::process;
use crate::config::Config;


//------------ Server --------------------------------------------------------

pub struct Server {
    config: Config,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Server {
            config
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn run(self) -> Result<(), ExitError> {
        Ok(())
    }
}



//------------ ExitError -----------------------------------------------------

pub struct ExitError;

impl ExitError {
    pub fn exit(self) -> ! {
        process::exit(1)
    }
}

