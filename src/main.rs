mod confirmation;
mod github;
mod mailgun;
mod team_api;
mod zulip;

use crate::confirmation::Confirmation;
use crate::github::SyncGitHub;
use crate::team_api::TeamApi;
use anyhow::Context;
use log::{error, info, warn};

const AVAILABLE_SERVICES: &[&str] = &["github", "mailgun", "zulip"];
const USER_AGENT: &str = "rust-lang teams sync (https://github.com/rust-lang/sync-team)";

fn usage() {
    eprintln!("available services:");
    for service in AVAILABLE_SERVICES {
        eprintln!("  {service}");
    }
    eprintln!("available flags:");
    eprintln!("  --help                   Show this help message");
    eprintln!("  --live                   Apply the proposed changes to the services");
    eprintln!("  --team-repo <path>       Path to the local team repo to use");
    eprintln!("  --only-print-plan        Print the execution plan without executing it");
    eprintln!("  --require-confirmation   Require external confirmation before applying changes");
    eprintln!("environment variables:");
    eprintln!("  GITHUB_TOKEN          Authentication token with GitHub");
    eprintln!("  GITHUB_IGNORED_ORGS   Space-separated list of orgs not to synchronize");
    eprintln!("  MAILGUN_API_TOKEN     Authentication token with Mailgun");
    eprintln!("  EMAIL_ENCRYPTION_KEY  Key used to decrypt encrypted emails in the team repo");
    eprintln!("  ZULIP_USERNAME        Username of the Zulip bot");
    eprintln!("  ZULIP_API_TOKEN       Authentication token of the Zulip bot");
    eprintln!("require-confirmation environment variables:");
    eprintln!("  CONFIRMATION_STREAM          Zulip stream to post confirmation messages on");
    eprintln!("  CONFIRMATION_TOPIC           Zulip topic to post confirmation messages on");
    eprintln!("  CONFIRMATION_BASE_URL        Base URL to the endpoint verifying the confirmation");
    eprintln!("  CONFIRMATION_APPROVED_HASH   Approved hash to apply");
    eprintln!("  CONFIRMATION_APPROVER        Identifier of the person approving the change");
}

fn app() -> anyhow::Result<()> {
    let mut dry_run = true;
    let mut next_team_repo = false;
    let mut only_print_plan = false;
    let mut require_confirmation = false;
    let mut team_repo = None;
    let mut services = Vec::new();
    for arg in std::env::args().skip(1) {
        if next_team_repo {
            team_repo = Some(arg);
            next_team_repo = false;
            continue;
        }
        match arg.as_str() {
            "--live" => dry_run = false,
            "--team-repo" => next_team_repo = true,
            "--help" => {
                usage();
                return Ok(());
            }
            "--only-print-plan" => only_print_plan = true,
            "--require-confirmation" => require_confirmation = true,
            service if AVAILABLE_SERVICES.contains(&service) => services.push(service.to_string()),
            _ => {
                eprintln!("unknown argument: {arg}");
                usage();
                std::process::exit(1);
            }
        }
    }
    if only_print_plan && require_confirmation {
        anyhow::bail!("you can only set one of --only-print-plan or --require-confirmation");
    }

    let team_api = team_repo
        .map(|p| TeamApi::Local(p.into()))
        .unwrap_or(TeamApi::Production);

    if services.is_empty() {
        info!("no service to synchronize specified, defaulting to all services");
        services = AVAILABLE_SERVICES
            .iter()
            .map(|s| (*s).to_string())
            .collect();
    }

    if dry_run {
        warn!("sync-team is running in dry mode, no changes will be applied.");
        warn!("run the binary with the --live flag to apply the changes.");
    }

    let mut diffs = Vec::new();
    for service in services {
        info!("synchronizing {}", service);
        match service.as_str() {
            "github" => {
                let ignored_orgs_tmp;
                let ignored_orgs = if let Ok(orgs) = get_env("GITHUB_IGNORED_ORGS") {
                    ignored_orgs_tmp = orgs;
                    ignored_orgs_tmp.split(' ').collect::<Vec<_>>()
                } else {
                    Vec::new()
                };

                let token = get_env("GITHUB_TOKEN")?;
                let sync = SyncGitHub::new(token, &team_api, &ignored_orgs, dry_run)?;
                diffs.push(ServiceDiff::GitHub {
                    diff: sync.diff_all()?,
                    sync,
                });
            }
            "mailgun" => {
                let token = get_env("MAILGUN_API_TOKEN")?;
                let encryption_key = get_env("EMAIL_ENCRYPTION_KEY")?;
                mailgun::run(&token, &encryption_key, &team_api, dry_run)?;
            }
            "zulip" => {
                let username = get_env("ZULIP_USERNAME")?;
                let token = get_env("ZULIP_API_TOKEN")?;
                zulip::run(username, token, &team_api, dry_run)?;
            }
            _ => panic!("unknown service: {service}"),
        }
    }

    for diff in &diffs {
        match diff {
            ServiceDiff::GitHub { diff, .. } => {
                info!("GitHub diff:\n{diff}");
            }
        }
    }

    if only_print_plan {
        // Nothing
    } else if require_confirmation {
        Confirmation::new(diffs)?.run()?;
    } else {
        run_diffs(diffs)?;
    }

    Ok(())
}

fn run_diffs(diffs: Vec<ServiceDiff>) -> anyhow::Result<()> {
    for diff in diffs {
        match diff {
            ServiceDiff::GitHub { sync, diff } => diff.apply(&sync)?,
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
enum ServiceDiff {
    GitHub {
        #[serde(skip)]
        sync: SyncGitHub,
        diff: github::Diff,
    },
}

fn get_env(key: &str) -> anyhow::Result<String> {
    std::env::var(key).with_context(|| format!("failed to get the {key} environment variable"))
}

fn main() {
    init_log();
    if let Err(err) = app() {
        error!("{}", err);
        for cause in err.chain() {
            error!("caused by: {}", cause);
        }
        std::process::exit(1);
    }
}

fn init_log() {
    let mut env = env_logger::Builder::new();
    env.filter_module("sync_team", log::LevelFilter::Info);
    if let Ok(content) = std::env::var("RUST_LOG") {
        env.parse_filters(&content);
    }
    env.init();
}
