# Cargo Upload

## Description
A cargo subcommand for publishing compressed crate to private registry (For example [Crates-Registry](https://gitlab.com/TalRoni/crates-registry)).\
In order to publish crates to a private registry you want to download the crate and it's dependencies (you can use `cargo download`) then you can upload the files to your private registry with this subcommand.

## Installation
cargo-upload can be install with cargo:
```bash
$ cargo install cargo-upload
```
## Usage
First config your private registry in the `.cargo/config` file
```toml
[registries]
my-registry = { index = "https://my-intranet:8080/git/index" }
```
See [Registries](https://doc.rust-lang.org/cargo/reference/registries.html) in the rust book for more information.

The command below can upload single crate.
```bash
cargo upload --registry my-registry crate-file.crate
```
## Roadmap
In the future we want to integrate this subcommand to the cargo repository.

## License
The license GNU GENERAL PUBLIC LICENSE Version 3
