use anyhow::Result;
use std::path::PathBuf;
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct Package {
  pub name: String,
  pub version: String,
}

#[derive(Debug, Deserialize, PartialEq, Clone)]
struct CargoToml {
  #[allow(dead_code)]
  pub package: Package,
}

pub fn get_package_id(file_path: PathBuf) -> Result<Package> {
  let content = std::fs::read_to_string(file_path)?;

  let cargo_file: CargoToml = toml::from_str(content.as_str())?;

  return Ok(cargo_file.package);
}