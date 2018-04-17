extern crate clap;
extern crate failure;
extern crate futures;
extern crate nvim_client_api;
extern crate tokio_core;

use clap::{App, Arg};
use failure::Error;
use futures::future::Future;
use nvim_client_api::NvimClient;
use tokio_core::reactor::Core;

fn do_main() -> Result<(), Error> {
    let matches = App::new("nvim_client")
        .version("0.1.0")
        .author("Joe Neeman <joeneeman@gmail.com>")
        .about("Sends commands to a running instance of neovim")
        .arg(Arg::with_name("eval")
             .short("e")
             .long("eval")
             .value_name("CMD")
             .help("Evaluates the given command in a running instance of neovim")
             .takes_value(true)
             .required(true))
        .arg(Arg::with_name("servername")
             .value_name("SERVER")
             .help("The name of the neovim server to connect to")
             .takes_value(true)
             .required(true))
        .get_matches();

    let mut core = Core::new()?;
    let handle = core.handle();
    let client = NvimClient::from_unix_socket(matches.value_of("servername").unwrap(), &handle)?;

    let client_task = client.eval(matches.value_of("eval").unwrap().to_owned())
        .and_then(|response| {
            println!("Got response: {:?}", response);
            Ok(())
        });

    core.run(client_task)?;
    Ok(())
}

fn main() {
    do_main().unwrap();
}

