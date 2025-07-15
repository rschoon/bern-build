use std::{ffi::OsString, fs, io::BufWriter, path::PathBuf, process::Command, sync::{Arc, Mutex}};

use anyhow::{bail, Context as _};
use minijinja::{value::Object, Value};

use crate::template;

#[derive(Default, Debug, Clone)]
pub struct BernConfig {
    pub stage_dir: PathBuf,
    pub file: PathBuf,
    pub context_root: PathBuf,
    pub docker_args: Vec<String>,
    pub output: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct Runtime {
    output: Mutex<Option<PathBuf>>,
}

impl Runtime {
    fn configure(&self, name: &str, value: Value) -> anyhow::Result<()> {
        match name {
            "output" => *self.output.lock().unwrap() = value.as_str().map(PathBuf::from),
            _ => bail!("Invalid parameter {name}")
        }

        Ok(())
    }
}

impl Object for Runtime {
    fn get_value(self: &Arc<Self>, key: &minijinja::Value) -> Option<Value> {
        let key = key.as_str()?;
        let this = self.clone();
        match key {
            "configure" => Some(Value::from_function(move |name: &str, value: Value| convert_result(this.configure(name, value)))),
            _ => None,
        }
    }

}

fn convert_result<I>(result: anyhow::Result<I>) -> Result<Value, minijinja::Error>
where 
    I: Into<Value>
{
    match result {
        Ok(v) => Ok(v.into()),
        Err(e) => Err(minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string()))
    }
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

    pub fn build(&self) -> anyhow::Result<()> {
        let df_path: PathBuf = self.config.stage_dir.join("Dockerfile");
        let df_file = BufWriter::new(fs::File::create(&df_path).with_context(|| format!("Failed to write file: {}", df_path.display()))?);

        self.jenv.render_to(&self.config.file, df_file)?;
        
        let mut command = Command::new("docker");
        command.arg("buildx")
            .arg("build").arg("-f").arg(&df_path)
            .args(&self.config.docker_args);

        let output = self.config.output.clone().or_else(|| self.runtime.output.lock().unwrap().clone());
        if let Some(output) = output {
            let mut output_arg = OsString::from("type=local,dest=");
            output_arg.push(output.as_os_str());

            command.arg("--output").arg(output_arg);
        }

        command.arg(&self.config.context_root);
        let status = command.status()?;

        if !status.success() {
            bail!("Build failed with {status}")
        }

        Ok(())
    }
}
