use std::{collections::HashMap, ffi::OsString, fs, io::{self, BufRead, BufWriter}, path::{Path, PathBuf}, process::Command, sync::{Arc, LazyLock, Mutex}};

use anyhow::{bail, Context as _};
use minijinja::{value::Object, Value};

use crate::template;

#[derive(Default, Debug, Clone)]
pub struct BernConfig {
    pub stage_dir: PathBuf,
    pub file: PathBuf,
    pub context_root: PathBuf,
    pub docker_args: Vec<String>,
    pub docker_tags: Vec<String>,
    pub build_args: HashMap<String, String>,
    pub targets: Vec<String>,
    pub output: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct RuntimeInner {
    config: Arc<BernConfig>,
    output: Option<PathBuf>,
    build_args: HashMap<String, String>,
    docker_tags: Vec<String>,
}

#[derive(Debug, Default)]
struct Runtime(Mutex<RuntimeInner>);

impl Runtime {
    fn set_build_arg(&self, name: &str, value: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().build_args.insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn build_arg(&self, name: &str) -> Option<String> {
        let inner = self.0.lock().unwrap();
        if let Some(outer) = inner.config.build_args.get(name) {
            Some(outer.to_owned())
        } else {
            inner.build_args.get(name).cloned()
        }
    }

    fn set_output(&self, output: Option<PathBuf>) {
        self.0.lock().unwrap().output = output;
    }

    fn add_docker_tag(&self, tag: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().docker_tags.push(tag.to_owned());
        Ok(())
    }
}

impl Object for Runtime {
    fn call_method(
        self: &Arc<Self>,
        state: &minijinja::State<'_, '_>,
        method: &str,
        args: &[Value],
    ) -> Result<Value, minijinja::Error> {
        let this = self.clone();
        let method = if method == "set_output" {
            Value::from_function(move |s: Option<&str>| this.set_output(s.map(PathBuf::from)))
        } else if method == "set_build_arg" {
            Value::from_function(move |k: &str, v: &str| mj_res(this.set_build_arg(k, v)))
        } else if method == "build_arg" {
            Value::from_function(move |k: &str| this.build_arg(k))
        } else if method == "add_docker_tag" {
            Value::from_function(move |t: &str| mj_res(this.add_docker_tag(t)))
        } else {
            return Err(minijinja::Error::from(minijinja::ErrorKind::UnknownMethod))
        };

        method.call(state, args)
    }
}

fn mj_res<I>(result: anyhow::Result<I>) -> Result<Value, minijinja::Error>
where 
    I: Into<Value>
{
    match result {
        Ok(v) => Ok(v.into()),
        Err(e) => Err(minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string()))
    }
}

fn docker_cmd() -> Result<&'static Path, anyhow::Error> {
    static DOCKER_CMD: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
        let candidates = [
            std::env::var_os("DOCKER"),
            Some("docker".into()),
            Some("podman".into())
        ];

        candidates.iter()
            .filter_map(|p| p.as_ref().and_then(|p2| which::which(p2).ok()))
            .next()
    });

    DOCKER_CMD.as_deref().ok_or_else(|| anyhow::anyhow!("docker or podman was not found in PATH"))
}

pub struct BernBuild {
    config: Arc<BernConfig>,
    runtime: Arc<Runtime>,
    jenv: template::Environment,
}

struct BuildTarget<'s> {
    name: Option<&'s str>,
    last: bool,
}

impl BernBuild {
    pub fn new(config: BernConfig) -> Self {
        let config = Arc::new(config);
        let runtime = Arc::new(Runtime::default());
        runtime.0.lock().unwrap().config = config.clone();

        let mut jenv = template::Environment::new(&config.context_root);
        jenv.set("bern".to_owned(), minijinja::Value::from_dyn_object(runtime.clone()));

        Self {
            config,
            runtime,
            jenv
        }
    }

    fn build_args(&self) -> Vec<String> {
        let rt = self.runtime.0.lock().unwrap();
        rt.build_args.iter().chain(self.config.build_args.iter()).map(|(k,v)| format!("{k}={v}")).collect()
    }

    fn docker_tags(&self) -> Vec<String> {
        let rt = self.runtime.0.lock().unwrap();
        self.config.docker_tags.iter().chain(rt.docker_tags.iter()).cloned().collect()
    }

    fn output(&self) -> Option<PathBuf> {
        let rt = self.runtime.0.lock().unwrap();
        self.config.output.clone().or_else(|| rt.output.clone())
    }

    pub fn render_to<W>(&self, writer: W) -> anyhow::Result<()>
    where
        W: std::io::Write
    {
        self.jenv.render_to(&self.config.file, writer)?;

        Ok(())
    }

    fn build_targets(&self) -> impl Iterator<Item=BuildTarget<'_>> {
        use itertools::Either;

        if self.config.targets.is_empty() {
            Either::Left([BuildTarget {
                name: None,
                last: true,
            }; 1].into_iter())
        } else {
            let count = self.config.targets.len();
            Either::Right(
                self.config.targets.iter().enumerate().map(move |t| {
                    BuildTarget {
                        name: Some(t.1),
                        last: t.0 == count - 1
                    }
                })
            )
        }
    }

    pub fn build(&self) -> anyhow::Result<()> {
        let df_path: PathBuf = self.config.stage_dir.join("Dockerfile");
        let df_file = BufWriter::new(fs::File::create(&df_path).with_context(|| format!("Failed to write file: {}", df_path.display()))?);

        self.render_to(df_file)?;

        let mut docker_tags = self.docker_tags().into_iter();
        let first_docker_tag = docker_tags.next();

        for target in self.build_targets() {
            let mut command = Command::new(docker_cmd()?);
            command.arg("buildx")
                .arg("build").arg("-f").arg(&df_path)
                .args(&self.config.docker_args);

            for build_arg in self.build_args() {
                command.arg("--build-arg").arg(build_arg);
            }

            if let Some(output) = self.output() {
                let mut output_arg = OsString::from("type=local,dest=");
                output_arg.push(output.as_os_str());

                command.arg("--output").arg(output_arg);
            }

            if let Some(name) = target.name {
                command.arg("--target").arg(name);
            }

            if target.last {
                if let Some(docker_tag) = first_docker_tag.as_deref() {
                    command.arg("-t").arg(docker_tag);
                }
            }

            command.arg(&self.config.context_root);
            let status = command.status()?;

            if !status.success() {
                bail!("Build failed with {status}")
            }
        }

        for tag in docker_tags {
            Command::new(docker_cmd()?)
                .arg("tag")
                .arg(first_docker_tag.as_ref().unwrap())
                .arg(&tag)
                .spawn()?;
        }

        Ok(())
    }

    pub fn push(&self) -> anyhow::Result<()> {
        let docker_tags = self.docker_tags();
        if docker_tags.is_empty() {
            bail!("Tag not set");
        } else {
            for tag in docker_tags {
                let mut command = Command::new(docker_cmd()?);
                command.arg("push").arg(&tag);

                let status = command.status()?;
                if !status.success() {
                    bail!("Tag push for {tag} failed with {status}")
                }
            }

            Ok(())
        }
    }

    fn read_dockerignore(&self) -> Vec<glob::Pattern> {
        let Ok(f) = fs::File::open(self.config.context_root.join(".dockerignore")) else { return Vec::new() };

        io::BufReader::new(f).lines()
            .map_while(|a| a.ok())
            .map(|a| a.trim().to_string())
            .filter(|a| !a.starts_with("#") && !a.is_empty())
            .filter_map(|a| glob::Pattern::new(&a).ok())
            .collect()
    }

    pub fn export_context(&self, w: impl io::Write) -> anyhow::Result<()> {
        let globs = self.read_dockerignore();

        let mut tar = tar::Builder::new(w);

        let mut dockerfile = Vec::new();
        self.render_to(io::Cursor::new(&mut dockerfile))
            .with_context(|| anyhow::anyhow!("Failed to render dockerfile"))?;

        let mut header = tar::Header::new_gnu();
        header.set_path("Dockerfile")?;
        header.set_size(dockerfile.len() as u64);
        header.set_cksum();

        tar.append(&header, &dockerfile as &[u8])
            .with_context(|| anyhow::anyhow!("Failed to write dockerfile to tar"))?;

        for entry in walkdir::WalkDir::new(&self.config.context_root) {
            let entry = entry?;
            let path = entry.path();
            if globs.iter().any(|g| g.matches_path(path)) {
                continue;
            }
            
            tar.append_path(path)
                .with_context(|| anyhow::anyhow!("Failed to write {} to tar", path.display()))?;
        }

        tar.finish()?;

        Ok(())
    }
}
