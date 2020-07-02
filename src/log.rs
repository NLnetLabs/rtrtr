use std::process;


pub struct Failed;

//------------ ExitError -----------------------------------------------------

pub struct ExitError;

impl ExitError {
    pub fn exit(self) -> ! {
        process::exit(1)
    }
}

