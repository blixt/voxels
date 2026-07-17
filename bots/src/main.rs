use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::PathBuf;
use voxels_bots::{BotLayout, BotRunConfig, run_bots};

struct Cli {
    config: BotRunConfig,
    report_path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = parse_args(std::env::args().skip(1))?;
    let report = run_bots(cli.config).await?;
    let json = serde_json::to_vec_pretty(&report)?;
    if let Some(path) = cli.report_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create report directory {}", parent.display()))?;
        }
        std::fs::write(&path, &json)
            .with_context(|| format!("write bot report {}", path.display()))?;
    } else {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&json)?;
        stdout.write_all(b"\n")?;
    }
    Ok(())
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> Result<Cli> {
    let mut world_url = None;
    let mut presence_url = None;
    let mut origin = "http://127.0.0.1:5173".to_owned();
    let mut subprotocol = "voxels.world.v17".to_owned();
    let mut auth_token = None;
    let mut bots = 4_usize;
    let mut duration_seconds = 10.0_f64;
    let mut seed = 0x5eed_cafe_u64;
    let mut layout = BotLayout::Mixed;
    let mut report_path = None;
    for argument in arguments {
        let (name, value) = argument
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("expected --name=value, received {argument}"))?;
        match name {
            "--world-url" => world_url = Some(value.to_owned()),
            "--presence-url" => presence_url = Some(value.to_owned()),
            "--origin" => origin = value.to_owned(),
            "--subprotocol" => subprotocol = value.to_owned(),
            "--auth-token" => auth_token = Some(value.to_owned()),
            "--bots" => bots = value.parse().context("parse --bots")?,
            "--duration-seconds" => {
                duration_seconds = value.parse().context("parse --duration-seconds")?;
            }
            "--seed" => seed = value.parse().context("parse --seed")?,
            "--layout" => {
                layout = match value {
                    "dense" => BotLayout::Dense,
                    "mixed" => BotLayout::Mixed,
                    _ => bail!("--layout must be dense or mixed"),
                };
            }
            "--report" => report_path = Some(PathBuf::from(value)),
            _ => bail!("unknown bot option {name}"),
        }
    }
    Ok(Cli {
        config: BotRunConfig {
            world_url: world_url.context("missing --world-url")?,
            presence_url: presence_url.context("missing --presence-url")?,
            origin,
            subprotocol,
            auth_token: auth_token.context("missing --auth-token")?,
            bots,
            duration_seconds,
            seed,
            layout,
        },
        report_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_reproducible_run_configuration() -> Result<()> {
        let cli = parse_args([
            "--world-url=ws://127.0.0.1:9777/v17/world".to_owned(),
            "--presence-url=ws://127.0.0.1:9777/v17/presence".to_owned(),
            "--auth-token=secret".to_owned(),
            "--bots=64".to_owned(),
            "--duration-seconds=30".to_owned(),
            "--seed=42".to_owned(),
            "--layout=dense".to_owned(),
            "--report=target/bots.json".to_owned(),
        ])?;
        assert_eq!(cli.config.bots, 64);
        assert_eq!(cli.config.duration_seconds, 30.0);
        assert_eq!(cli.config.seed, 42);
        assert_eq!(cli.config.layout, BotLayout::Dense);
        assert_eq!(cli.report_path, Some(PathBuf::from("target/bots.json")));
        Ok(())
    }
}
