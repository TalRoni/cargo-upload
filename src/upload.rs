use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::Path;
use std::str;
use std::task::Poll;

use anyhow::{anyhow, bail, format_err, Context as _, Result};
use cargo::core::dependency::DepKind;
use cargo::core::manifest::ManifestMetadata;
use cargo::core::{Dependency, Package, QueryKind, Source, SourceId, Workspace};
use cargo::sources::{RegistrySource, SourceConfigMap, CRATES_IO_DOMAIN, CRATES_IO_REGISTRY};
use cargo::util::auth::{self, Secret};
use cargo::util::network::http::http_handle;
use cargo::util::{Config, IntoUrl};
use cargo_util::paths;
use crates_io::{self, NewCrate, NewCrateDependency, Registry};
use flate2::read::GzDecoder;
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle, ProgressFinish};
use itertools::Itertools;
use log::{info, warn};
use tar::Archive;
use tempfile::TempDir;

use crate::UploadOpts;

fn progress_bar(size: usize) -> ProgressBar {
    ProgressBar::new(size as u64)
        .with_style(
            ProgressStyle::with_template(
                "{spinner:.green} {msg} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})",
                )
                .expect("template is correct")
                .progress_chars("#>-"),
        )
        .with_finish(ProgressFinish::AndLeave)
}

pub async fn upload(opts: UploadOpts) -> Result<()> {
    let crate_paths = opts.crate_paths.iter().cloned().filter(|p| p.ends_with(".crate")).collect_vec();
    info!("Upload {} crates to {} index", crate_paths.len(), opts.index.clone().unwrap_or(String::from("crate.io")));

    let pb = progress_bar(crate_paths.len());
    let tasks = futures::stream::iter(crate_paths.into_iter())
        .map(|crate_path| {
            let pb = pb.clone();
            let opts = opts.clone();
            tokio::spawn(async move {
                upload_crate(&crate_path, &opts)?;
                pb.inc(1);
                Ok::<_, anyhow::Error>(())
            })
        })
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;

    for t in tasks {
        match t.unwrap() {
            Ok(_) => {}
            Err(err) => {
                if opts.keep_going {
                    warn!("Can't upload crate: {}", err);
                } else {
                    return Err(err);
                }
            }
        }
    }
    pb.finish();
    Ok(())
}

pub fn upload_crate(crate_path: impl AsRef<Path>, opts: &UploadOpts) -> Result<()> {
    let config = cargo::Config::default()?;
    config.shell().set_verbosity(cargo::core::Verbosity::Quiet);
    let tar_gz = File::open(&crate_path)?;
    let tar = GzDecoder::new(tar_gz);
    let mut krate = Archive::new(tar);
    let directory = TempDir::new()?;
    krate.unpack(directory.path())?;
    let crate_folder = std::fs::read_dir(directory.path())?
        .exactly_one()
        .context("There is more then one folder")??;
    let manifest_path = crate_folder.path().join("Cargo.toml");
    let ws = Workspace::new(&manifest_path, &config)?;
    let pkg = ws
        .members()
        .exactly_one()
        .map_err(|e| anyhow!("Packages error: {}", e))?;
    let mut publish_registry = opts.registry.clone();
    if let Some(ref allowed_registries) = *pkg.publish() {
        if publish_registry.is_none() && allowed_registries.len() == 1 {
            // If there is only one allowed registry, push to that one directly,
            // even though there is no registry specified in the command.
            let default_registry = &allowed_registries[0];
            if default_registry != CRATES_IO_REGISTRY {
                // Don't change the registry for crates.io and don't warn the user.
                // crates.io will be defaulted even without this.
                log::info!(
                    "Found `{}` as only allowed registry. Publishing to it automatically.",
                    default_registry
                );
                publish_registry = Some(default_registry.clone());
            }
        }

        let reg_name = publish_registry
            .clone()
            .unwrap_or_else(|| CRATES_IO_REGISTRY.to_string());
        if allowed_registries.is_empty() {
            bail!(
                "`{}` cannot be published.\n\
                 `package.publish` is set to `false` or an empty list in Cargo.toml and prevents publishing.",
                pkg.name(),
            );
        } else if !allowed_registries.contains(&reg_name) {
            bail!(
                "`{}` cannot be published.\n\
                 The registry `{}` is not listed in the `package.publish` value in Cargo.toml.",
                pkg.name(),
                reg_name
            );
        }
    }
    // This is only used to confirm that we can create a token before we build the package.
    // This causes the credential provider to be called an extra time, but keeps the same order of errors.
    let ver = pkg.version().to_string();
    let mutation = auth::Mutation::PrePublish;

    let (mut registry, reg_ids) = registry(
        &config,
        opts.token.as_deref().map(Secret::from),
        opts.index.as_deref(),
        publish_registry.as_deref(),
        true,
        Some(mutation).filter(|_| !opts.dry_run),
    )?;
    verify_dependencies(pkg, &registry, reg_ids.original)?;

    let tarball = File::open(crate_path)?;
    if !opts.dry_run {
        let hash = cargo_util::Sha256::new()
            .update_file(&tarball)?
            .finish_hex();
        let mutation = Some(auth::Mutation::Publish {
            name: pkg.name().as_str(),
            vers: &ver,
            cksum: &hash,
        });
        registry.set_token(Some(auth::auth_token(
            &config,
            &reg_ids.original,
            None,
            mutation,
        )?));
    }

    transmit(
        &config,
        pkg,
        &tarball,
        &mut registry,
        reg_ids.original,
        opts.dry_run,
    )?;
    if !opts.dry_run {
        const DEFAULT_TIMEOUT: u64 = 60;
        let timeout = if config.cli_unstable().publish_timeout {
            let timeout: Option<u64> = config.get("publish.timeout")?;
            timeout.unwrap_or(DEFAULT_TIMEOUT)
        } else {
            DEFAULT_TIMEOUT
        };
        if 0 < timeout {
            let timeout = std::time::Duration::from_secs(timeout);
            wait_for_publish(&config, reg_ids.original, pkg, timeout)?;
        }
    }

    Ok(())
}

/// Returns true if the dependency is either git or path, false otherwise
/// Error if a git/path dep is transitive, but has no version (registry source).
/// This check is performed on dependencies before publishing or packaging
fn check_dep_has_version(dep: &Dependency, publish: bool) -> Result<bool> {
    let which = if dep.source_id().is_path() {
        "path"
    } else if dep.source_id().is_git() {
        "git"
    } else {
        return Ok(false);
    };

    if !dep.specified_req() && dep.is_transitive() {
        let dep_version_source = dep.registry_id().map_or_else(
            || CRATES_IO_DOMAIN.to_string(),
            |registry_id| registry_id.display_registry_name(),
        );
        anyhow::bail!(
            "all dependencies must have a version specified when {}.\n\
             dependency `{}` does not specify a version\n\
             Note: The {} dependency will use the version from {},\n\
             the `{}` specification will be removed from the dependency declaration.",
            if publish { "publishing" } else { "packaging" },
            dep.package_name(),
            if publish { "published" } else { "packaged" },
            dep_version_source,
            which,
        )
    }
    Ok(true)
}

fn verify_dependencies(pkg: &Package, registry: &Registry, registry_src: SourceId) -> Result<()> {
    for dep in pkg.dependencies().iter() {
        if check_dep_has_version(dep, true)? {
            continue;
        }
        // TomlManifest::prepare_for_publish will rewrite the dependency
        // to be just the `version` field.
        if dep.source_id() != registry_src {
            if !dep.source_id().is_registry() {
                // Consider making SourceId::kind a public type that we can
                // exhaustively match on. Using match can help ensure that
                // every kind is properly handled.
                panic!("unexpected source kind for dependency {:?}", dep);
            }
            // Block requests to send to crates.io with alt-registry deps.
            // This extra hostname check is mostly to assist with testing,
            // but also prevents someone using `--index` to specify
            // something that points to crates.io.
            if registry_src.is_crates_io() || registry.host_is_crates_io() {
                bail!("crates cannot be published to crates.io with dependencies sourced from other\n\
                       registries. `{}` needs to be published to crates.io before publishing this crate.\n\
                       (crate `{}` is pulled from {})",
                      dep.package_name(),
                      dep.package_name(),
                      dep.source_id());
            }
        }
    }
    Ok(())
}

fn transmit(
    config: &Config,
    pkg: &Package,
    tarball: &File,
    registry: &mut Registry,
    registry_id: SourceId,
    dry_run: bool,
) -> Result<()> {
    let deps = pkg
        .dependencies()
        .iter()
        .filter(|dep| {
            // Skip dev-dependency without version.
            dep.is_transitive() || dep.specified_req()
        })
        .map(|dep| {
            // If the dependency is from a different registry, then include the
            // registry in the dependency.
            let dep_registry_id = match dep.registry_id() {
                Some(id) => id,
                None => SourceId::crates_io(config)?,
            };
            // In the index and Web API, None means "from the same registry"
            // whereas in Cargo.toml, it means "from crates.io".
            let dep_registry = if dep_registry_id != registry_id {
                Some(dep_registry_id.url().to_string())
            } else {
                None
            };

            Ok(NewCrateDependency {
                optional: dep.is_optional(),
                default_features: dep.uses_default_features(),
                name: dep.package_name().to_string(),
                features: dep.features().iter().map(|s| s.to_string()).collect(),
                version_req: dep.version_req().to_string(),
                target: dep.platform().map(|s| s.to_string()),
                kind: match dep.kind() {
                    DepKind::Normal => "normal",
                    DepKind::Build => "build",
                    DepKind::Development => "dev",
                }
                .to_string(),
                registry: dep_registry,
                explicit_name_in_toml: dep.explicit_name_in_toml().map(|s| s.to_string()),
            })
        })
        .collect::<Result<Vec<NewCrateDependency>>>()?;
    let manifest = pkg.manifest();
    let ManifestMetadata {
        ref authors,
        ref description,
        ref homepage,
        ref documentation,
        ref keywords,
        ref readme,
        ref repository,
        ref license,
        ref license_file,
        ref categories,
        ref badges,
        ref links,
        ref rust_version,
    } = *manifest.metadata();
    let readme_content = readme.as_ref().and_then(|readme| {
        paths::read(&pkg.root().join(readme))
            .with_context(|| format!("failed to read `readme` file for package `{}`", pkg))
            .ok()
    });
    if let Some(ref file) = *license_file {
        if !pkg.root().join(file).exists() {
            bail!("the license file `{}` does not exist", file)
        }
    }

    // Do not upload if performing a dry run
    if dry_run {
        config.shell().warn("aborting upload due to dry run")?;
        return Ok(());
    }

    let string_features = match manifest.original().features() {
        Some(features) => features
            .iter()
            .map(|(feat, values)| {
                (
                    feat.to_string(),
                    values.iter().map(|fv| fv.to_string()).collect(),
                )
            })
            .collect::<BTreeMap<String, Vec<String>>>(),
        None => BTreeMap::new(),
    };

    let warnings = registry
        .publish(
            &NewCrate {
                name: pkg.name().to_string(),
                vers: pkg.version().to_string(),
                deps,
                features: string_features,
                authors: authors.clone(),
                description: description.clone(),
                homepage: homepage.clone(),
                documentation: documentation.clone(),
                keywords: keywords.clone(),
                categories: categories.clone(),
                readme: readme_content,
                readme_file: readme.clone(),
                repository: repository.clone(),
                license: license.clone(),
                license_file: license_file.clone(),
                badges: badges.clone(),
                links: links.clone(),
                rust_version: rust_version.clone(),
            },
            tarball,
        )
        .with_context(|| format!("failed to publish to registry at {}", registry.host()))?;

    if !warnings.invalid_categories.is_empty() {
        let msg = format!(
            "the following are not valid category slugs and were \
             ignored: {}. Please see https://crates.io/category_slugs \
             for the list of all category slugs. \
             ",
            warnings.invalid_categories.join(", ")
        );
        config.shell().warn(&msg)?;
    }

    if !warnings.invalid_badges.is_empty() {
        let msg = format!(
            "the following are not valid badges and were ignored: {}. \
             Either the badge type specified is unknown or a required \
             attribute is missing. Please see \
             https://doc.rust-lang.org/cargo/reference/manifest.html#package-metadata \
             for valid badge types and their required attributes.",
            warnings.invalid_badges.join(", ")
        );
        config.shell().warn(&msg)?;
    }

    if !warnings.other.is_empty() {
        for msg in warnings.other {
            config.shell().warn(&msg)?;
        }
    }

    Ok(())
}

fn wait_for_publish(
    config: &Config,
    registry_src: SourceId,
    pkg: &Package,
    timeout: std::time::Duration,
) -> Result<()> {
    let version_req = format!("={}", pkg.version());
    let mut source = SourceConfigMap::empty(config)?.load(registry_src, &HashSet::new())?;
    let source_description = source.describe();
    let query = Dependency::parse(pkg.name(), Some(&version_req), registry_src)?;

    let now = std::time::Instant::now();
    let sleep_time = std::time::Duration::from_secs(1);
    loop {
        {
            let _lock = config.acquire_package_cache_lock()?;
            // Force re-fetching the source
            //
            // As pulling from a git source is expensive, we track when we've done it within the
            // process to only do it once, but we are one of the rare cases that needs to do it
            // multiple times
            config
                .updated_sources()
                .remove(&source.replaced_source_id());
            source.invalidate_cache();
            let summaries = loop {
                // Exact to avoid returning all for path/git
                match source.query_vec(&query, QueryKind::Exact) {
                    std::task::Poll::Ready(res) => {
                        break res?;
                    }
                    std::task::Poll::Pending => source.block_until_ready()?,
                }
            };
            if !summaries.is_empty() {
                break;
            }
        }

        if timeout < now.elapsed() {
            config.shell().warn(format!(
                "timed out waiting for `{}` to be in {}",
                pkg.name(),
                source_description
            ))?;
            break;
        }
        std::thread::sleep(sleep_time);
    }

    Ok(())
}

/// Returns the `Registry` and `Source` based on command-line and config settings.
///
/// * `token_from_cmdline`: The token from the command-line. If not set, uses the token
///   from the config.
/// * `index`: The index URL from the command-line.
/// * `registry`: The registry name from the command-line. If neither
///   `registry`, or `index` are set, then uses `crates-io`.
/// * `force_update`: If `true`, forces the index to be updated.
/// * `token_required`: If `true`, the token will be set.
fn registry(
    config: &Config,
    token_from_cmdline: Option<Secret<&str>>,
    index: Option<&str>,
    registry: Option<&str>,
    force_update: bool,
    token_required: Option<auth::Mutation<'_>>,
) -> Result<(Registry, RegistrySourceIds)> {
    let source_ids = get_source_id(config, index, registry)?;

    if token_required.is_some() && index.is_some() && token_from_cmdline.is_none() {
        bail!("command-line argument --index requires --token to be specified");
    }
    if let Some(token) = token_from_cmdline {
        auth::cache_token(config, &source_ids.original, token);
    }

    let cfg = {
        let _lock = config.acquire_package_cache_lock()?;
        let mut src = RegistrySource::remote(source_ids.replacement, &HashSet::new(), config)?;
        // Only update the index if `force_update` is set.
        if force_update {
            src.invalidate_cache()
        }
        let cfg = loop {
            match src.config()? {
                Poll::Pending => src
                    .block_until_ready()
                    .with_context(|| format!("failed to update {}", source_ids.replacement))?,
                Poll::Ready(cfg) => break cfg,
            }
        };
        cfg.expect("remote registries must have config")
    };
    let api_host = cfg
        .api
        .ok_or_else(|| format_err!("{} does not support API commands", source_ids.replacement))?;
    let token = if token_required.is_some() || cfg.auth_required {
        Some(auth::auth_token(
            config,
            &source_ids.original,
            None,
            token_required,
        )?)
    } else {
        None
    };
    let handle = http_handle(config)?;
    Ok((
        Registry::new_handle(api_host, token, handle, cfg.auth_required),
        source_ids,
    ))
}

/// Gets the SourceId for an index or registry setting.
///
/// The `index` and `reg` values are from the command-line or config settings.
/// If both are None, and no source-replacement is configured, returns the source for crates.io.
/// If both are None, and source replacement is configured, returns an error.
///
/// The source for crates.io may be GitHub, index.crates.io, or a test-only registry depending
/// on configuration.
///
/// If `reg` is set, source replacement is not followed.
///
/// The return value is a pair of `SourceId`s: The first may be a built-in replacement of
/// crates.io (such as index.crates.io), while the second is always the original source.
fn get_source_id(
    config: &Config,
    index: Option<&str>,
    reg: Option<&str>,
) -> Result<RegistrySourceIds> {
    let sid = match (reg, index) {
        (None, None) => SourceId::crates_io(config)?,
        (_, Some(i)) => SourceId::for_registry(&i.into_url()?)?,
        (Some(r), None) => SourceId::alt_registry(config, r)?,
    };
    // Load source replacements that are built-in to Cargo.
    let builtin_replacement_sid = SourceConfigMap::empty(config)?
        .load(sid, &HashSet::new())?
        .replaced_source_id();
    let replacement_sid = SourceConfigMap::new(config)?
        .load(sid, &HashSet::new())?
        .replaced_source_id();
    if reg.is_none() && index.is_none() && replacement_sid != builtin_replacement_sid {
        // Neither --registry nor --index was passed and the user has configured source-replacement.
        if let Some(replacement_name) = replacement_sid.alt_registry_key() {
            bail!("crates-io is replaced with remote registry {replacement_name};\ninclude `--registry {replacement_name}` or `--registry crates-io`");
        } else {
            bail!("crates-io is replaced with non-remote-registry source {replacement_sid};\ninclude `--registry crates-io` to use crates.io");
        }
    } else {
        Ok(RegistrySourceIds {
            original: sid,
            replacement: builtin_replacement_sid,
        })
    }
}

struct RegistrySourceIds {
    /// Use when looking up the auth token, or writing out `Cargo.lock`
    original: SourceId,
    /// Use when interacting with the source (querying / publishing , etc)
    ///
    /// The source for crates.io may be replaced by a built-in source for accessing crates.io with
    /// the sparse protocol, or a source for the testing framework (when the replace_crates_io
    /// function is used)
    ///
    /// User-defined source replacement is not applied.
    replacement: SourceId,
}
