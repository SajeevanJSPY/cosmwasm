use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::exit;

use anyhow::Context;
use clap::{Arg, ArgAction, Command};
use colored::Colorize;

use cosmwasm_vm::internals::{
    check_wasm_with_limits, compile, make_compiling_engine, LogOutput, Logger,
};
use cosmwasm_vm::{capabilities_from_csv, Config, WasmLimits};

const DEFAULT_AVAILABLE_CAPABILITIES: &str =
    "iterator,staking,stargate,cosmwasm_1_1,cosmwasm_1_2,cosmwasm_1_3,cosmwasm_1_4,cosmwasm_2_0,cosmwasm_2_1";

pub fn main() {
    let matches = Command::new("Contract checking")
        .version(env!("CARGO_PKG_VERSION"))
        .long_about("Checks the given wasm file (memories, exports, imports, available capabilities, and non-determinism).")
        .author("Mauro Lacy <mauro@lacy.com.es>")
        .arg(
            Arg::new("CAPABILITIES")
                // `long` setting required to turn the position argument into an option 🤷
                .long("available-capabilities")
                .aliases(["FEATURES", "supported-features"]) // Old names
                .value_name("CAPABILITIES")
                .help("Sets the available capabilities that the desired target chain has")
                .num_args(1)
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("VERBOSE")
                .long("verbose")
                .num_args(0)
                .help("Prints additional information on stderr")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("CONFIG")
            .long("wasm-config")
            .value_name("CONFIG_FILE")
            .conflicts_with("CAPABILITIES")
            .help("Provide a file with the chain's Wasmd configuration.")
            .long_help("Provide a file with the chain's Wasmd configuration.
You can query this configuration from the chain, using the WasmConfig query.
If this is not provided, the default values are used. This conflicts with the --available-capabilities flag because the config also contains those.")
            .num_args(1)
            .action(ArgAction::Set)
        )
        .arg(
            Arg::new("WASM")
                .help("Wasm file to read and compile")
                .required(true)
                .index(1)
                .num_args(0..)
                .action(ArgAction::Append),
        )
        .get_matches();

    let config_file = matches.get_one::<String>("CONFIG");
    let available_capabilities_csv = matches
        .get_one::<String>("CAPABILITIES")
        .map(|s| s.as_str());

    // Available capabilities and Wasm limits
    let (wasm_limits, available_capabilities) = match (config_file, available_capabilities_csv) {
        (Some(config_file), _) => {
            let config = read_config(config_file).unwrap();
            (config.wasm_limits, config.cache.available_capabilities)
        }
        (_, available_capabilities_csv) => {
            let available_capabilities = capabilities_from_csv(
                available_capabilities_csv.unwrap_or(DEFAULT_AVAILABLE_CAPABILITIES),
            );
            (WasmLimits::default(), available_capabilities)
        }
    };
    println!("Available capabilities: {available_capabilities:?}");
    println!();

    // File
    let paths = matches
        .get_many::<String>("WASM")
        .expect("Error parsing file names");

    let (passes, failures): (Vec<_>, _) = paths
        .map(|p| {
            let result = check_contract(
                p,
                &available_capabilities,
                matches.get_flag("VERBOSE"),
                &wasm_limits,
            );
            match &result {
                Ok(_) => println!("{}: {}", p, "pass".green()),
                Err(e) => {
                    println!("{}: {}", p, "failure".red());
                    println!("{e}");
                }
            };
            result
        })
        .partition(|result| result.is_ok());
    println!();

    if failures.is_empty() {
        println!(
            "All contracts ({}) {} checks!",
            passes.len(),
            "passed".green()
        );
    } else {
        println!(
            "{}: {}, {}: {}",
            "Passes".green(),
            passes.len(),
            "failures".red(),
            failures.len()
        );
        exit(1);
    }
}

fn read_config(path: &str) -> anyhow::Result<Config> {
    let file = File::open(path).context("error opening config file")?;
    let config = rmp_serde::from_read(file).context("error parsing config file")?;
    Ok(config)
}

fn check_contract(
    path: &str,
    available_capabilities: &HashSet<String>,
    verbose: bool,
    wasm_limits: &WasmLimits,
) -> anyhow::Result<()> {
    let mut file = File::open(path)?;

    // Read wasm
    let mut wasm = Vec::<u8>::new();
    file.read_to_end(&mut wasm)?;

    // Potentially lossy filename or path as used as a short prefix for the output
    let filename_identifier: String = Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or(path.to_string());
    let prefix = format!("    {}: ", filename_identifier);
    let logs = if verbose {
        Logger::On {
            prefix: &prefix,
            output: LogOutput::StdErr,
        }
    } else {
        Logger::Off
    };
    // Check wasm
    check_wasm_with_limits(&wasm, available_capabilities, wasm_limits, logs)?;

    // Compile module
    let engine = make_compiling_engine(None);
    let _module = compile(&engine, &wasm)?;

    Ok(())
}
