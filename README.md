
# bern builds

bern is a simple tool to improve docker usability as a build system by providing Jinja2 based templating on top of the Dockerfile format.

## Installation

At present binary artifacts are not available, so it must be built and installed from source.

## Basic Usage

The default name for the input file is `Dockerfile.j2`, and the context directory (files available to the build) is the same directory the input file is located in.

The `Dockerfile.j2` file is a [Dockerfile](https://docs.docker.com/reference/dockerfile/) which utilizes Jinja2 syntax to allow additional flexibility in defining the build.  The specific implementation for Jinja2 syntax is the [minijinja](https://docs.rs/minijinja/latest/minijinja/syntax/) library, which supports most Jinja2 features.

Multi-stage builds, which is a native feature of docker, provides a significant amount of flexibility.  If a stage is given a target name when defined (for example, `name` in `FROM src AS name`), then that target can be selected by passing it via the `--target name` flag.  Multiple targets can be specified, and if no target is specified, then the last target will be run.

Tag names can be applied to the resulting docker image via the `-t` flag, which can be provided multiple times.  Alternatively, files can be exported from the build by using the `--output` flag, which can be combined with a scratch image to output specific results of the build.  For example:

```
FROM rust:latest AS builder

MKDIR /src
COPY . /src
RUN cargo build

FROM scratch AS build-output
COPY /src/target/release/result-binary /

```

```
$ bern --output output
[...]
$ ls output
result-binary
```