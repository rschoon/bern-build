use std::{fs, io::BufWriter, path::PathBuf, process::Command};

use anyhow::Context as _;
use clap::Parser;

mod template;

#[derive(Clone, Debug, Parser)]
struct Cli {
    #[clap(long, short, default_value = "Dockerfile.j2")]
    file: PathBuf,

    #[clap(long)]
    docker_args: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    let context_root = ".";

    let jenv = template::Environment::new(context_root);
    let stage_dir = tempfile::tempdir()?;

    let df_path: PathBuf = stage_dir.path().join("Dockerfile");
    let df_file = BufWriter::new(fs::File::create(&df_path).with_context(|| format!("Failed to write file: {}", df_path.display()))?);

    jenv.render_to(&args.file, df_file)?;

    // TODO: docker_args is probably wrong... eg expected to use --docker-args="--build-arg hello"
    Command::new("docker").arg("buildx").arg("build").arg("-f").arg(&df_path).args(&args.docker_args).arg(context_root);

    Ok(())
}
