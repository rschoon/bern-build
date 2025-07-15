use std::path::PathBuf;

use clap::Parser;

mod build;
mod template;

#[derive(Clone, Debug, Parser)]
struct Cli {
    #[clap(long, short, default_value = "Dockerfile.j2")]
    file: PathBuf,

    #[clap(long)]
    docker_args: Vec<String>,

    #[clap(long)]
    output: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();

    let stage_dir = tempfile::tempdir()?;
    let build = build::BernBuild::new(build::BernConfig {
        stage_dir: stage_dir.path().to_owned(),
        file: args.file,
        context_root: PathBuf::from("."),
        // TODO: docker_args is probably wrong... eg expected to use --docker-args="--build-arg hello"
        docker_args: args.docker_args,
        output: args.output,
    });

    build.build()?;

    Ok(())
}
