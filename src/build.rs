use std::{ffi::OsString, fs, io::BufWriter, path::{Path, PathBuf}, process::Command, sync::{Arc, LazyLock, Mutex}};

use anyhow::{bail, Context as _};
use minijinja::{value::Object, Value};

use crate::template;

#[derive(Default, Debug, Clone)]
pub struct BernConfig {
    pub stage_dir: PathBuf,
    pub file: PathBuf,
    pub context_root: PathBuf,
    pub docker_args: Vec<String>,
    pub docker_tag: Option<String>,
    pub build_args: Vec<String>,
    pub output: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct RuntimeInner {
    output: Option<PathBuf>,
    build_args: Vec<String>,
}

#[derive(Debug, Default)]
struct Runtime(Mutex<RuntimeInner>);

impl Runtime {
    fn set_build_arg(&self, name: &str, value: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().build_args.push(format!("{name}={value}"));
        Ok(())
    }

    fn set_output(&self, output: Option<PathBuf>) {
        self.0.lock().unwrap().output = output;
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
    config: BernConfig,
    runtime: Arc<Runtime>,
    jenv: template::Environment,
}

impl BernBuild {
    pub fn new(config: BernConfig) -> Self {
        let runtime = Arc::new(Runtime::default());
        let mut jenv = template::Environment::new(&config.context_root);
        jenv.set("bern".to_owned(), minijinja::Value::from_dyn_object(runtime.clone()));

        Self {
            config,
            runtime,
            jenv
        }
    }

    fn docker_tag(&self) -> Option<&str> {
        self.config.docker_tag.as_deref()
    }

    pub fn render_to<W>(&self, writer: W) -> anyhow::Result<()>
    where
        W: std::io::Write
    {
        self.jenv.render_to(&self.config.file, writer)?;

        Ok(())
    }

    pub fn build(&self) -> anyhow::Result<()> {
        let df_path: PathBuf = self.config.stage_dir.join("Dockerfile");
        let df_file = BufWriter::new(fs::File::create(&df_path).with_context(|| format!("Failed to write file: {}", df_path.display()))?);

        self.render_to(df_file)?;

        let mut command = Command::new(docker_cmd()?);
        command.arg("buildx")
            .arg("build").arg("-f").arg(&df_path)
            .args(&self.config.docker_args);

        let runtime = self.runtime.0.lock().unwrap();

        for build_arg in runtime.build_args.iter().chain(self.config.build_args.iter()) {
            command.arg("--build-arg").arg(build_arg);
        }

        let output = self.config.output.clone().or_else(|| runtime.output.clone());
        if let Some(output) = output {
            let mut output_arg = OsString::from("type=local,dest=");
            output_arg.push(output.as_os_str());

            command.arg("--output").arg(output_arg);
        }

        if let Some(docker_tag) = self.docker_tag() {
            command.arg("-t").arg(docker_tag);
        }

        command.arg(&self.config.context_root);
        let status = command.status()?;

        if !status.success() {
            bail!("Build failed with {status}")
        }

        Ok(())
    }

    pub fn push(&self) -> anyhow::Result<()> {
        if let Some(docker_tag) = self.docker_tag() {
            let mut command = Command::new(docker_cmd()?);
            command.arg("push").arg(docker_tag);

            let status = command.status()?;
            if !status.success() {
                bail!("Tag push failed with {status}")
            }

            Ok(())
        } else {
            bail!("Tag not set");
        }
    }
}
