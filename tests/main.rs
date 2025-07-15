use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CommandCargoExt;
use predicates::prelude::*;
use rstest::rstest;
use serde::Deserialize;
use tempfile::TempDir;
use std::{collections::HashMap, io::Read, path::{Path, PathBuf}, process::Command};
use std::ffi::OsString;

#[derive(Debug, Deserialize)]
struct TestData {
    #[serde(default)]
    setup: TestSetup,
    run: Vec<TestRun>
}

#[derive(Default, Debug, Deserialize)]
struct TestSetup {
    files: Vec<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TestRun {
    args: Vec<String>,
    status_code: i32,
    stderr_contains: Vec<String>,
    verify_files: HashMap<PathBuf, TestFileVerify>,
}

#[derive(Debug, Deserialize)]
enum TestFileVerify {
    #[serde(rename="content")]
    Content(String),
}

fn show_file(path: &Path) {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Can't show {path:?}: {e}");
            return;
        }
    };
    let mut buffer = String::new();
    f.read_to_string(&mut buffer).unwrap();
    eprintln!("{}: {:?}", path.display(), buffer);
}

fn list_files(path: &Path) -> impl Iterator<Item=PathBuf> {
    let w = walkdir::WalkDir::new(path);
    w.into_iter().filter_map(|s| {
        s.ok().and_then(|e| (!e.file_type().is_dir()).then(|| e.into_path()))
    })
}

#[rstest]
fn main(
    #[files("tests/data/**/*.toml")] path: PathBuf
) {
    let parent = path.parent().unwrap();
    let mut f = std::fs::File::open(&path).unwrap();
    let mut toml_data = String::new();
    f.read_to_string(&mut toml_data).unwrap();
    let test: TestData = toml::from_str(&toml_data).unwrap();

    let temp_dir = TempDir::new().unwrap();
    
    let mut auto_args: Vec<OsString> = Vec::new();
    let tpl_path = path.with_extension("j2");
    if tpl_path.exists() {
        let tpl_path_dest = temp_dir.path().join(tpl_path.file_name().unwrap());
        std::fs::copy(&tpl_path, &tpl_path_dest).unwrap();

        auto_args.push("-f".into());
        auto_args.push(tpl_path_dest.into())
    }

    for add_file in &test.setup.files {
        std::fs::copy(parent.join(add_file), temp_dir.path().join(add_file)).unwrap();
    }

    if test.run.is_empty() {
        panic!("No test runs defined");
    }

    for (idx, run) in test.run.iter().enumerate() {
        eprintln!("--- {idx}");

        let mut command = Command::cargo_bin("bern").unwrap();
        command.args(&run.args);
        command.args(&auto_args);
        command.current_dir(temp_dir.path());
        let mut cmd_assert = command.assert();
        
        let files: Vec<_> = list_files(temp_dir.path()).map(|p| p.display().to_string()).collect();

        eprintln!("Stdout: {}", String::from_utf8_lossy(&cmd_assert.get_output().stdout));
        eprintln!("Stderr: {}", String::from_utf8_lossy(&cmd_assert.get_output().stderr));
        eprintln!("Files: {}", files.join(", "));

        cmd_assert = cmd_assert.code(predicate::eq(run.status_code));
        for s in &run.stderr_contains {
            cmd_assert = cmd_assert.stderr(predicate::str::contains(s));
        }

        check_files(temp_dir.path(), run);
    }
}

fn check_files(temp_dir: &Path, run: &TestRun) {
    for (result_name, verify) in &run.verify_files {
        let result_file = temp_dir.join(result_name);

        let success = match verify {
            TestFileVerify::Content(content) => {
                let predicate_file = predicate::eq(content.as_ref()).from_file_path();
                predicate_file.eval(result_file.as_path())
            }
        };

        if !success {
            eprintln!("File for contents for {:?} do not match expected!", &result_file);
            show_file(&result_file);
            panic!("Check failed!")
        }
    }
}
