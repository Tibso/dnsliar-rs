mod commands;
mod functions;
mod redis_mod;

use crate::commands::{Cli, Commands, Subcommands};

use std::{fs, process::ExitCode};
use clap::Parser;
use redis::Client;
use serde::{Serialize, Deserialize};

/// The configuration file structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Confile {
    daemon_id: String,
    redis_address: String
}

fn main() -> ExitCode {
    // Arguments are parsed and stored in the Cli struct
    let cli = Cli::parse();

    // First argument should be the path_to_confile,
    // which is read to build the configuration file
    let confile: Confile = {
        // Reads the file into a big String
        let tmp_string = match fs::read_to_string(&cli.path_to_confile) {
            Ok(ok) => ok,
            Err(err) => {
                println!("Error reading file from: {:?}: {:?}", cli.path_to_confile, err);
                // Returns with CONFIG exitcode on error
                return ExitCode::from(78)
            }
        };
        // Deserializes the JSON String
        match serde_json::from_str(&tmp_string) {
            Ok(ok) => ok,
            Err(err) => {
                println!("Error deserializing config file data: {:?}", err);
                // Returns with CONFIG exitcode on error
                return ExitCode::from(78)
            }
        }
    };
    
    // A client is built and probes the Redis server to check its availability
    let client = match Client::open(format!("redis://{}/", confile.redis_address)) {
        Ok(ok) => ok,
        Err(err) => {
            println!("Error probing the Redis server: {:?}", err);
            // Returns with NOHOST exitcode on error
            return ExitCode::from(68)
        }
    };
    // A connection is created using the client
    let connection = match client.get_connection() {
        Ok(ok) => ok,
        Err(err) => {
            println!("Error creating the connection: {:?}", err);
            // Returns with UNAVAILABLE exitcode on error
            return ExitCode::from(69) // NICE
        }
    };

    // Second argument should be the command to use
    // Each element of the "Commands" enum calls its own function
    let result = match &cli.command {
        Commands::ShowConf {}
            => functions::show_conf(
                connection, confile
            ),
        Commands::EditConf (subcommand)
            => match subcommand {
                Subcommands::AddBinds {binds}
                    => functions::add_binds(
                        connection, confile.daemon_id, binds.to_owned()
                    ),
                Subcommands::ClearParam {parameter}
                    => functions::clear_parameter(
                        connection, confile.daemon_id, parameter.to_owned()
                    ),
                Subcommands::Forwarders {forwarders}
                    => functions::set_forwarders(
                        connection, confile.daemon_id, forwarders.to_owned()
                    ),
                Subcommands::BlackholeIps {blackhole_ips}
                    => functions::set_blackhole_ips(
                        connection, confile.daemon_id, blackhole_ips.to_owned()
                    ),
                Subcommands::BlockIps {blocked_ips}
                    => functions::add_blocked_ips(
                        connection, confile.daemon_id, blocked_ips.to_owned()
                    )
            },
        Commands::ClearStats {pattern}
            => functions::clear_stats(
                connection, confile.daemon_id, pattern.to_owned()
            ),
        Commands::Stats {pattern}
            => functions::get_stats(
                connection, confile.daemon_id, pattern.to_owned()
            ),
        Commands::GetInfo {matchclass}
            => functions::get_info(
                connection, matchclass.to_owned()
            ),
        Commands::Drop {pattern}
            => functions::drop_matchclasses(
                connection, pattern.to_owned()
            ),
        Commands::Feed {path_to_list, matchclass}
            => functions::feed_matchclass(
                connection, path_to_list.to_owned(), matchclass.to_owned()
            ),
        Commands::SetRule {matchclass, qtype, ip}
            => functions::set_rule(
                connection, matchclass.to_owned(), qtype.to_owned(), ip.to_owned()
            ),
        Commands::DelRule {matchclass, qtype}
            => functions::delete_rule(
                connection, matchclass.to_owned(), qtype.to_owned()
            )
    };

    // Returns both result variants
    match result {
        Ok(exitcode) => exitcode,
        // Converts errors to UNAVAILABLE exitcode
        Err(_) => ExitCode::from(69)
    }
}
