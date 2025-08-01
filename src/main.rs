use std::{fs, io::{self, BufWriter}, path::PathBuf};

use clap::{Parser, Subcommand};

mod build;
mod template;

#[derive(Clone, Debug, Parser)]
#[clap(version)]
struct Cli {
    /// Docker template file
    #[clap(long, short, default_value = "Dockerfile.j2")]
    file: PathBuf,

    /// Additional docker arguments (multiple)
    #[clap(long)]
    docker_args: Vec<String>,

    /// Build arguments (multiple)
    #[clap(long, short('b'))]
    build_arg: Vec<String>,

    /// Push resulting docker image
    #[clap(long)]
    push: bool,

    /// Tag resulting docker image (multiple)
    #[clap(long, short('t'))]
    tag: Vec<String>,

    /// Targets to build (multiple)
    #[clap(long)]
    target: Vec<String>,

    /// Output path to export contents of final target
    #[clap(long)]
    output: Option<PathBuf>,

    #[clap(subcommand)]
    command: Option<BernCommand>,
}

#[derive(Clone, Debug, Subcommand)]
enum BernCommand {
    /// Print out resulting Dockerfile
    ShowDockerfile,
    /// Export context as a tar without building
    ExportContext {
        destination: PathBuf,
    }
}

fn transform_docker_args(args: Vec<String>) -> Vec<String> {
    args.iter().flat_map(|a| shlex::split(a).unwrap_or_default()).collect()
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();

    let stage_dir = tempfile::tempdir()?;
    let build_args = args.build_arg.into_iter()
        .map(|a| {
            match a.split_once('=') {
                Some((k, v)) => (k.to_owned(), v.to_owned()),
                None => (a, String::new())
            }
        
        })
        .collect();
    let build = build::BernBuild::new(build::BernConfig {
        stage_dir: stage_dir.path().to_owned(),
        file: args.file,
        context_root: PathBuf::from("."),
        docker_args: transform_docker_args(args.docker_args),
        docker_tags: args.tag,
        build_args,
        targets: args.target,
        output: args.output,
    });

    match args.command {
        Some(BernCommand::ShowDockerfile) => {
            build.render_to(std::io::stdout())?;
            Ok(())
        },
        Some(BernCommand::ExportContext { destination }) => {
            let output: Box<dyn io::Write> = if destination.as_os_str() == "-" {
                Box::new(std::io::stdout())
            } else {
                Box::new(BufWriter::new(fs::File::create(destination)?))
            };

            build.export_context(output)?;

            Ok(())
        },
        None => {
            build.build()?;

            if args.push {
                build.push()?;
            }

            Ok(())
        },
    }
}
