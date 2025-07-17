use std::path::PathBuf;

use clap::Parser;

mod build;
mod template;

#[derive(Clone, Debug, Parser)]
struct Cli {
    #[clap(long, short, default_value = "Dockerfile.j2")]
    file: PathBuf,

    #[clap(long)]
    debug: bool,

    #[clap(long)]
    docker_args: Vec<String>,

    #[clap(long)]
    build_arg: Vec<String>,

    #[clap(long)]
    push: bool,

    #[clap(long, short)]
    tag: Option<String>,

    #[clap(long)]
    output: Option<PathBuf>,
}

fn transform_docker_args(args: Vec<String>) -> Vec<String> {
    args.iter().flat_map(|a| shlex::split(a).unwrap_or_default()).collect()
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();

    let stage_dir = tempfile::tempdir()?;
    let build = build::BernBuild::new(build::BernConfig {
        stage_dir: stage_dir.path().to_owned(),
        file: args.file,
        context_root: PathBuf::from("."),
        docker_args: transform_docker_args(args.docker_args),
        docker_tag: args.tag,
        build_args: args.build_arg,
        output: args.output,
    });

    if args.debug {
        build.render_to(std::io::stdout())?;

        return Ok(())
    }

    build.build()?;

    if args.push {
        build.push()?;
    }

    Ok(())
}
