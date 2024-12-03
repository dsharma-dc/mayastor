//!
//! methods to interact with NVMe controllers

use super::context::Context;
use crate::{context::OutputFormat, GrpcStatus};
use clap::{ArgMatches, Command};
use colored_json::ToColoredJson;
use io_engine_api::v0 as rpc;
use snafu::ResultExt;
use std::convert::TryFrom;
use tonic::Status;

pub fn subcommands() -> Command {
    let list = Command::new("list").about("List existing NVMe controllers");
    let stats = Command::new("stats").about("Display I/O statistics for NVMe controllers");

    Command::new("controller")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .about("NVMe controllers")
        .subcommand(list)
        .subcommand(stats)
}

pub async fn handler(ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    match matches.subcommand().unwrap() {
        ("list", args) => list_controllers(ctx, args).await,
        ("stats", args) => controller_stats(ctx, args).await,
        (cmd, _) => {
            Err(Status::not_found(format!("command {cmd} does not exist"))).context(GrpcStatus)
        }
    }
}

fn controller_state_to_str(idx: i32) -> String {
    match rpc::NvmeControllerState::try_from(idx).unwrap() {
        rpc::NvmeControllerState::New => "new",
        rpc::NvmeControllerState::Initializing => "init",
        rpc::NvmeControllerState::Running => "running",
        rpc::NvmeControllerState::Faulted => "faulted",
        rpc::NvmeControllerState::Unconfiguring => "unconfiguring",
        rpc::NvmeControllerState::Unconfigured => "unconfigured",
    }
    .to_string()
}

async fn controller_stats(mut ctx: Context, _matches: &ArgMatches) -> crate::Result<()> {
    let response = ctx
        .client
        .stat_nvme_controllers(rpc::Null {})
        .await
        .context(GrpcStatus)?;

    match ctx.output {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&response.get_ref())
                    .unwrap()
                    .to_colored_json_auto()
                    .unwrap()
            );
        }
        OutputFormat::Default => {
            let controllers = &response.get_ref().controllers;
            if controllers.is_empty() {
                ctx.v1("No NVMe controllers found");
                return Ok(());
            }

            let table: Vec<Vec<String>> = controllers
                .iter()
                .map(|c| {
                    let stats = c.stats.as_ref().unwrap();

                    let num_read_ops = stats.num_read_ops.to_string();
                    let num_write_ops = stats.num_write_ops.to_string();
                    let bytes_read = stats.bytes_read.to_string();
                    let bytes_written = stats.bytes_written.to_string();

                    vec![
                        c.name.to_string(),
                        num_read_ops,
                        num_write_ops,
                        bytes_read,
                        bytes_written,
                    ]
                })
                .collect();

            let hdr = vec!["NAME", "READS", "WRITES", "READ/B", "WRITTEN/B"];
            ctx.print_list(hdr, table);
        }
    }

    Ok(())
}

async fn list_controllers(mut ctx: Context, _matches: &ArgMatches) -> crate::Result<()> {
    let response = ctx
        .client
        .list_nvme_controllers(rpc::Null {})
        .await
        .context(GrpcStatus)?;

    match ctx.output {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&response.get_ref())
                    .unwrap()
                    .to_colored_json_auto()
                    .unwrap()
            );
        }
        OutputFormat::Default => {
            let controllers = &response.get_ref().controllers;
            if controllers.is_empty() {
                ctx.v1("No NVMe controllers found");
                return Ok(());
            }

            let table = controllers
                .iter()
                .map(|c| {
                    let size = c.size.to_string();
                    let blk_size = c.blk_size.to_string();
                    let state = controller_state_to_str(c.state);

                    vec![c.name.clone(), size, state, blk_size]
                })
                .collect();

            let hdr = vec!["NAMEs", "SIZE", "STATE", "BLKSIZE"];
            ctx.print_list(hdr, table);
        }
    }

    Ok(())
}
