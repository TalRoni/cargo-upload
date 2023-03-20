# Cargo Upload

## Description
An expansion tool for cargo to upload compressed crate to private registry (For example [Crates-Registry](https://gitlab.com/TalRoni/crates-registry)).
In order to publish crates to a privates registry you want to download the crate and it's dependencies with `cargo vendor` then you can upload the files to a private registry with this expansion command.



## Badges
On some READMEs, you may see small images that convey metadata, such as whether or not all the tests are passing for the project. You can use Shields to add some to your README. Many services also have instructions for adding a badge.

## Installation
Install with cargo:
```
cargo install cargo-upload
```
## Usage
```
cargo upload --registry private-registry crate-file.crate
```

## Support


## Roadmap
In the future we want to merge this command to cargo repository

## Contributing

## License
The license GNU GENERAL PUBLIC LICENSE Version 3
