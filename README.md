# Cargo Upload

## Description
A cargo subcommand for publishing compressed a crate to a private registry (For example [Crates-Registry](https://github.com/TalRoni/crates-registry)).\
To publish crates to a private registry you want to download the crate and its dependencies (you can use [`cargo collect`](https://crates.io/crates/cargo-collect)) then you can upload the files to your private registry with this subcommand.

## Installation
cargo-upload can be installed via cargo:
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

The command below can upload a single crate.
```bash
cargo upload --registry my-registry crate-file.crate
```

The command below can upload all crates in a folder.
```bash
cargo upload --registry my-registry ./my-crates
```

Run `cargo upload --help` for more information.

## Roadmap
In the future, we want to integrate this subcommand into the cargo repository.

## License
The license GNU GENERAL PUBLIC LICENSE Version 3
