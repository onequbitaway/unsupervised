//! What two processes compare in the handshake: the world configuration
//! and the set of plugins.

use unsup_kernel::{Plugin, World, WorldConfig};

use super::error::ClusterError;
use super::wire::Hash;
use crate::durable::Plugins;

/// Returns the plugins as the kernel wants them.
pub(crate) fn plugin_refs(plugins: &Plugins) -> Vec<&dyn Plugin> {
    plugins.iter().map(AsRef::as_ref).collect()
}

/// The hashes that every process of one cluster must share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    /// Hash of the world configuration, without the list of hosted realms.
    pub world: Hash,
    /// Hash of the plugin names in their order and of the catalogue that
    /// they give: verbs with argument schemas, parameters with values,
    /// items, attributes and conditions.
    pub plugins: Hash,
}

/// Returns the configuration without the list of hosted realms: what all
/// processes of a cluster have in common.
pub(crate) fn common(config: &WorldConfig) -> WorldConfig {
    WorldConfig { hosted: None, ..config.clone() }
}

/// Computes the fingerprint of a process.
///
/// It also checks the configuration and the data of the plugins, so a
/// mistake there is an error here and not a panic later.
pub(crate) fn fingerprint(
    plugins: &Plugins,
    config: &WorldConfig,
) -> Result<Fingerprint, ClusterError> {
    let config_error = |e: &dyn std::fmt::Display| ClusterError::Config(e.to_string());
    let common = common(config);
    let bytes = rmp_serde::to_vec_named(&common).map_err(|e| config_error(&e))?;
    let world = Hash(*blake3::hash(&bytes).as_bytes());

    // A world without a realm has the definitions of all plugins and runs
    // no world generation.
    let probe = WorldConfig { hosted: Some(Vec::new()), ..common };
    let probe = World::try_new(&plugin_refs(plugins), probe).map_err(|e| config_error(&e))?;
    let catalogue = rmp_serde::to_vec_named(&probe.catalogue()).map_err(|e| config_error(&e))?;
    let mut hasher = blake3::Hasher::new();
    for plugin in plugins.iter() {
        let name = plugin.name().as_bytes();
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name);
    }
    hasher.update(&catalogue);
    Ok(Fingerprint { world, plugins: Hash(*hasher.finalize().as_bytes()) })
}
