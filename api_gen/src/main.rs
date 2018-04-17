extern crate clap;
extern crate failure;
extern crate itertools;
extern crate rmp_serde;
extern crate rmpv;
extern crate serde;

#[macro_use] extern crate serde_derive;

use clap::App;
use failure::Error;
use std::fs::File;
use std::io::{Read, Write};

mod api;
use api::{Api, ApiType};

fn main()
{
    do_main().unwrap();
}

fn do_main() -> Result<(), Error> {
    let matches = App::new("api_gen")
        .version("0.1")
        .author("Joe Neeman <joeneeman@gmail.com>")
        .about("generates a rust API for neovim's msgpack-rpc")
        .args_from_usage(
            "<API_SPEC>     'path to a file containing the output of `nvim --api-info`'
            <OUT>           'path to write out the API definitions'")
        .get_matches();

    let mut api_spec = File::open(matches.value_of("API_SPEC").unwrap())?;
    let mut buf = Vec::new();
    api_spec.read_to_end(&mut buf)?;

    let api: Api = rmp_serde::from_slice(&buf)?;
    let mut out = File::create(matches.value_of("OUT").unwrap())?;

    writeln!(out, "use futures::Future;")?;
    writeln!(out, "use rmp_rpc::Value;")?;
    writeln!(out, "use super::*;")?;

    writeln!(out, "impl NvimClient {{")?;
    for f in &api.functions {
        if f.deprecated_since.is_none() && !f.method {
            writeln!(out, "\t{}", f.macro_call("nvim_"))?;
        }
    }
    writeln!(out, "}}")?;

    for (name, ref decl) in &api.types {
        writeln!(out, "nvim_type!({});", name)?;
        writeln!(out, "impl<'client> {}<'client> {{", name)?;

        for f in &api.functions {
            if f.deprecated_since.is_none() && f.method && f.name.starts_with(decl.prefix) {
                writeln!(out, "\t{}", f.macro_call(decl.prefix))?;
            }
        }

        writeln!(out, "}}")?;
    }

    Ok(())
}

