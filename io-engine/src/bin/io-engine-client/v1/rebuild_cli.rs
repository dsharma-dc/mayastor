//!
//! methods to interact with the rebuild process

use crate::{
    context::{Context, OutputFormat},
    ClientError, GrpcStatus,
};
use clap::{Arg, ArgMatches, Command};
use colored_json::ToColoredJson;
use io_engine_api::v1;
use snafu::ResultExt;
use std::convert::TryFrom;
use tonic::Status;

pub async fn handler(ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    match matches.subcommand().unwrap() {
        ("start", args) => start(ctx, args).await,
        ("stop", args) => stop(ctx, args).await,
        ("pause", args) => pause(ctx, args).await,
        ("resume", args) => resume(ctx, args).await,
        ("state", args) => state(ctx, args).await,
        ("stats", args) => stats(ctx, args).await,
        ("progress", args) => progress(ctx, args).await,
        ("history", args) => history(ctx, args).await,
        (cmd, _) => {
            Err(Status::not_found(format!("command {cmd} does not exist"))).context(GrpcStatus)
        }
    }
}

pub fn subcommands() -> Command {
    let start = Command::new("start")
        .about("starts a rebuild")
        .arg(
            Arg::new("uuid")
                .required(true)
                .index(1)
                .help("uuid of the nexus"),
        )
        .arg(
            Arg::new("uri")
                .required(true)
                .index(2)
                .help("uri of child to start rebuilding"),
        );

    let stop = Command::new("stop")
        .about("stops a rebuild")
        .arg(
            Arg::new("uuid")
                .required(true)
                .index(1)
                .help("uuid of the nexus"),
        )
        .arg(
            Arg::new("uri")
                .required(true)
                .index(2)
                .help("uri of child to stop rebuilding"),
        );

    let pause = Command::new("pause")
        .about("pauses a rebuild")
        .arg(
            Arg::new("uuid")
                .required(true)
                .index(1)
                .help("uuid of the nexus"),
        )
        .arg(
            Arg::new("uri")
                .required(true)
                .index(2)
                .help("uri of child to pause rebuilding"),
        );

    let resume = Command::new("resume")
        .about("resumes a rebuild")
        .arg(
            Arg::new("uuid")
                .required(true)
                .index(1)
                .help("uuid of the nexus"),
        )
        .arg(
            Arg::new("uri")
                .required(true)
                .index(2)
                .help("uri of child to resume rebuilding"),
        );

    let state = Command::new("state")
        .about("gets the rebuild state of the child")
        .arg(
            Arg::new("uuid")
                .required(true)
                .index(1)
                .help("uuid of the nexus"),
        )
        .arg(
            Arg::new("uri")
                .required(true)
                .index(2)
                .help("uri of child to get the rebuild state from"),
        );

    let stats = Command::new("stats")
        .about("gets the rebuild stats of the child")
        .arg(
            Arg::new("uuid")
                .required(true)
                .index(1)
                .help("uuid of the nexus"),
        )
        .arg(
            Arg::new("uri")
                .required(true)
                .index(2)
                .help("uri of child to get the rebuild stats from"),
        );

    let progress = Command::new("progress")
        .about("shows the progress of a rebuild")
        .arg(
            Arg::new("uuid")
                .required(true)
                .index(1)
                .help("uuid of the nexus"),
        )
        .arg(
            Arg::new("uri")
                .required(true)
                .index(2)
                .help("uri of child to get the rebuild progress from"),
        );

    let history = Command::new("history")
        .about("shows the rebuild history for children of a nexus")
        .arg(
            Arg::new("uuid")
                .required(true)
                .index(1)
                .help("uuid of the nexus"),
        );

    Command::new("rebuild")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .about("Rebuild management")
        .subcommand(start)
        .subcommand(stop)
        .subcommand(pause)
        .subcommand(resume)
        .subcommand(state)
        .subcommand(stats)
        .subcommand(progress)
        .subcommand(history)
}

async fn start(mut ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    let uuid = matches
        .get_one::<String>("uuid")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uuid".to_string(),
        })?
        .to_string();
    let uri = matches
        .get_one::<String>("uri")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uri".to_string(),
        })?
        .to_string();

    let response = ctx
        .v1
        .nexus
        .start_rebuild(v1::nexus::StartRebuildRequest {
            nexus_uuid: uuid,
            uri: uri.clone(),
        })
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
            println!("{}", &uri);
        }
    };

    Ok(())
}

async fn stop(mut ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    let uuid = matches
        .get_one::<String>("uuid")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uuid".to_string(),
        })?
        .to_string();
    let uri = matches
        .get_one::<String>("uri")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uri".to_string(),
        })?
        .to_string();

    let response = ctx
        .v1
        .nexus
        .stop_rebuild(v1::nexus::StopRebuildRequest {
            nexus_uuid: uuid,
            uri: uri.clone(),
        })
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
            println!("{}", &uri);
        }
    };

    Ok(())
}

async fn pause(mut ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    let uuid = matches
        .get_one::<String>("uuid")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uuid".to_string(),
        })?
        .to_string();
    let uri = matches
        .get_one::<String>("uri")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uri".to_string(),
        })?
        .to_string();

    let response = ctx
        .v1
        .nexus
        .pause_rebuild(v1::nexus::PauseRebuildRequest {
            nexus_uuid: uuid,
            uri: uri.clone(),
        })
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
            println!("{}", &uri);
        }
    };

    Ok(())
}

async fn resume(mut ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    let uuid = matches
        .get_one::<String>("uuid")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uuid".to_string(),
        })?
        .to_string();
    let uri = matches
        .get_one::<String>("uri")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uri".to_string(),
        })?
        .to_string();

    let response = ctx
        .v1
        .nexus
        .resume_rebuild(v1::nexus::ResumeRebuildRequest {
            nexus_uuid: uuid,
            uri: uri.clone(),
        })
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
            println!("{}", &uri);
        }
    };

    Ok(())
}

async fn state(mut ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    let uuid = matches
        .get_one::<String>("uuid")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uuid".to_string(),
        })?
        .to_string();
    let uri = matches
        .get_one::<String>("uri")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uri".to_string(),
        })?
        .to_string();

    let response = ctx
        .v1
        .nexus
        .get_rebuild_state(v1::nexus::RebuildStateRequest {
            nexus_uuid: uuid,
            uri,
        })
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
            ctx.print_list(vec!["state"], vec![vec![response.get_ref().state.clone()]]);
        }
    };

    Ok(())
}

async fn history(mut ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    let uuid = matches
        .get_one::<String>("uuid")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uuid".to_string(),
        })?
        .to_string();
    let response = ctx
        .v1
        .nexus
        .get_rebuild_history(v1::nexus::RebuildHistoryRequest { uuid: uuid.clone() })
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
            let response = &response.get_ref();
            if response.records.is_empty() {
                return Ok(());
            }
            let table = response
                .records
                .iter()
                .map(|r| {
                    let state = rebuild_state_to_str(
                        v1::nexus::RebuildJobState::try_from(r.state).unwrap(),
                    )
                    .to_string();

                    vec![
                        r.child_uri.clone(),
                        r.src_uri.clone(),
                        r.blocks_total.to_string(),
                        r.blocks_transferred.to_string(),
                        state,
                        r.blocks_per_task.to_string(),
                        r.block_size.to_string(),
                        r.is_partial.to_string(),
                        r.start_time.as_ref().unwrap().to_string(),
                        r.end_time.as_ref().unwrap().to_string(),
                    ]
                })
                .collect();
            ctx.print_list(
                vec![
                    "CHILD",
                    "SOURCE",
                    ">TOTAL",
                    ">TRANSFERRED",
                    ">STATE",
                    ">BLK_PER_TASK",
                    ">BLK_SIZE",
                    ">PARTIAL",
                    "START",
                    "END",
                ],
                table,
            );
        }
    };

    Ok(())
}

async fn stats(mut ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    let uuid = matches
        .get_one::<String>("uuid")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uuid".to_string(),
        })?
        .to_string();
    let uri = matches
        .get_one::<String>("uri")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uri".to_string(),
        })?
        .to_string();

    ctx.v2(&format!(
        "Getting the rebuild stats of child {uri} on nexus {uuid}"
    ));
    let response = ctx
        .v1
        .nexus
        .get_rebuild_stats(v1::nexus::RebuildStatsRequest {
            nexus_uuid: uuid,
            uri: uri.clone(),
        })
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
            let response = &response.get_ref();
            ctx.print_list(
                vec![
                    ">TOTAL",
                    ">RECOVERED",
                    ">TRANSFERRED",
                    ">REMAINING",
                    ">PROGRESS (%)",
                    ">BLK_PER_TASK",
                    ">BLK_SIZE",
                    ">PARTIAL",
                    ">TASKS_TOTAL",
                    ">TASKS_ACTIVE",
                ],
                vec![vec![
                    response.blocks_total.to_string(),
                    response.blocks_recovered.to_string(),
                    response.blocks_transferred.to_string(),
                    response.blocks_remaining.to_string(),
                    response.progress.to_string(),
                    response.blocks_per_task.to_string(),
                    response.block_size.to_string(),
                    response.is_partial.to_string(),
                    response.tasks_total.to_string(),
                    response.tasks_active.to_string(),
                ]],
            );
        }
    };

    Ok(())
}

async fn progress(mut ctx: Context, matches: &ArgMatches) -> crate::Result<()> {
    let uuid = matches
        .get_one::<String>("uuid")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uuid".to_string(),
        })?
        .to_string();
    let uri = matches
        .get_one::<String>("uri")
        .ok_or_else(|| ClientError::MissingValue {
            field: "uri".to_string(),
        })?
        .to_string();

    let response = ctx
        .v1
        .nexus
        .get_rebuild_stats(v1::nexus::RebuildStatsRequest {
            nexus_uuid: uuid,
            uri: uri.clone(),
        })
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
            ctx.print_list(
                vec!["progress (%)"],
                vec![vec![response.get_ref().progress.to_string()]],
            );
        }
    };
    Ok(())
}

fn rebuild_state_to_str(s: v1::nexus::RebuildJobState) -> &'static str {
    match s {
        v1::nexus::RebuildJobState::Init => "init",
        v1::nexus::RebuildJobState::Rebuilding => "rebuilding",
        v1::nexus::RebuildJobState::Stopped => "stopped",
        v1::nexus::RebuildJobState::Paused => "paused",
        v1::nexus::RebuildJobState::Failed => "failed",
        v1::nexus::RebuildJobState::Completed => "completed",
    }
}
