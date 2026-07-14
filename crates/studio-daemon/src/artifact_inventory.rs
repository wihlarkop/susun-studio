//! Engine-wide artifact inventory (images, containers, build cache, and
//! registry capability) sourced entirely from the public Susun facade.
//! Everything here is read-only: nothing pulls, pushes, builds, tags, or
//! prunes. Mutating artifact workflows are Phase 15b+.

use susun::{ContainerEngine, ContainerId, DockerCompatibleEngine, ImageId, ProjectName};
use turso::Database;

use crate::susun_integration::enum_label;

/// Studio-owned selected-runtime attribution attached to every engine-wide
/// artifact response, so built-in and external runtimes stay distinguishable
/// and external runtimes are never presented as Studio-owned.
pub struct RuntimeContextRow {
    pub runtime_profile_id: Option<String>,
    pub runtime_class: Option<String>,
    pub display_name: Option<String>,
    pub is_selected: Option<bool>,
}

/// Resolves display-safe runtime attribution for the given profile. `None`
/// means the platform default engine (no profile selected or bound).
pub async fn runtime_context(db: &Database, profile_id: Option<&str>) -> RuntimeContextRow {
    let profile = match profile_id {
        Some(id) => crate::runtime::list_all_profiles(db)
            .await
            .ok()
            .and_then(|profiles| profiles.into_iter().find(|profile| profile.id == id)),
        None => None,
    };
    match profile {
        Some(profile) => RuntimeContextRow {
            runtime_profile_id: Some(profile.id),
            runtime_class: Some(profile.runtime_class),
            display_name: Some(profile.display_name),
            is_selected: Some(profile.is_selected),
        },
        None => RuntimeContextRow {
            runtime_profile_id: profile_id.map(str::to_owned),
            runtime_class: None,
            display_name: None,
            is_selected: None,
        },
    }
}

/// One Studio project's derived project identity, used only to associate
/// engine-wide containers with a known project. Never exposed on the wire.
struct KnownProjectRow {
    id: String,
    instance_id: String,
}

/// Re-derives each stored project's opaque instance ID the same way the SDK
/// derived it at import time (name + directory, hashed, path not retained),
/// so containers can be matched to a known Studio project without trusting
/// engine-reported labels or re-touching the filesystem.
async fn known_projects(db: &Database) -> Result<Vec<KnownProjectRow>, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query("SELECT id, name, path FROM projects", ())
        .await?;
    let mut known = Vec::new();
    while let Some(row) = rows.next().await? {
        let id: String = row.get(0)?;
        let name: String = row.get(1)?;
        let path: String = row.get(2)?;
        if name.trim().is_empty() || path.trim().is_empty() {
            continue;
        }
        let instance_id = susun::ProjectInstanceId::derive(&ProjectName::new(name), &path)
            .as_str()
            .to_owned();
        known.push(KnownProjectRow { id, instance_id });
    }
    Ok(known)
}

fn known_project_id(known: &[KnownProjectRow], project_identity: Option<&str>) -> Option<String> {
    let project_identity = project_identity?;
    known
        .iter()
        .find(|project| project.instance_id == project_identity)
        .map(|project| project.id.clone())
}

/// Display-safe engine-wide container summary, redaction-inherited from the
/// SDK's `EngineContainerSummary` (label values, endpoints, and paths are
/// already excluded there) plus Studio's own project association.
pub struct ContainerSummaryRow {
    pub id: String,
    pub name: String,
    pub state: String,
    pub health: Option<String>,
    pub image_reference: Option<String>,
    pub label_keys: Vec<String>,
    pub known_project_id: Option<String>,
    pub created_at_epoch_seconds: Option<u64>,
    pub writable_size_bytes: Option<u64>,
    pub root_filesystem_size_bytes: Option<u64>,
}

pub struct ContainerInventoryRow {
    pub observed_at_epoch_seconds: u64,
    pub containers: Vec<ContainerSummaryRow>,
}

/// Result of an engine-wide capability-gated read. `data` is `None` exactly
/// when the provider does not support the operation (or the call otherwise
/// failed) — never an HTTP error, so unsupported providers surface as an
/// explicit capability state rather than a generic failure.
pub struct CapabilityResult<T> {
    pub capability: String,
    pub data: Option<T>,
}

fn observed_image_reference(image: &susun::ObservedImageRef) -> Option<String> {
    match image {
        susun::ObservedImageRef::Id(id) => Some(id.as_str().to_owned()),
        susun::ObservedImageRef::Reference(reference) => Some(reference.as_str().to_owned()),
        susun::ObservedImageRef::Unknown => None,
    }
}

fn container_summary_row(
    container: &susun::EngineContainerSummary,
    known: &[KnownProjectRow],
) -> ContainerSummaryRow {
    ContainerSummaryRow {
        id: container.id.as_str().to_owned(),
        name: container.name.as_str().to_owned(),
        state: enum_label(container.state),
        health: container.health.map(enum_label),
        image_reference: observed_image_reference(&container.image),
        label_keys: container
            .label_keys
            .iter()
            .map(|key| key.as_str().to_owned())
            .collect(),
        known_project_id: known_project_id(
            known,
            container.project_identity.as_ref().map(|id| id.as_str()),
        ),
        created_at_epoch_seconds: container.created_at_epoch_seconds,
        writable_size_bytes: container.writable_size_bytes,
        root_filesystem_size_bytes: container.root_filesystem_size_bytes,
    }
}

/// Lists containers across the whole engine. `capability` reflects the
/// provider's advertised support for engine-wide container inventory.
pub async fn container_inventory(
    db: &Database,
    engine: &DockerCompatibleEngine,
) -> Result<CapabilityResult<ContainerInventoryRow>, String> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| error.to_string())?;
    let capability = enum_label(capabilities.supports_container_inventory);

    let data = match engine.container_inventory().await {
        Ok(inventory) => {
            let known = known_projects(db)
                .await
                .map_err(|error| error.to_string())?;
            Some(ContainerInventoryRow {
                observed_at_epoch_seconds: inventory.observed_at_epoch_seconds,
                containers: inventory
                    .containers
                    .iter()
                    .map(|container| container_summary_row(container, &known))
                    .collect(),
            })
        }
        Err(_) => None,
    };

    Ok(CapabilityResult { capability, data })
}

/// Reads one container from the engine-wide inventory.
pub async fn container_details(
    db: &Database,
    engine: &DockerCompatibleEngine,
    container_id: &str,
) -> Result<ContainerSummaryRow, String> {
    let id = ContainerId::new(container_id.to_owned())
        .map_err(|_| "container id must not be empty".to_owned())?;
    let container = engine
        .container_details(&id)
        .await
        .map_err(|error| error.to_string())?;
    let known = known_projects(db)
        .await
        .map_err(|error| error.to_string())?;
    Ok(container_summary_row(&container, &known))
}

/// Display-safe engine-wide image summary, redaction-inherited from the
/// SDK's `EngineImageSummary` (label values excluded there).
pub struct ImageSummaryRow {
    pub id: String,
    pub references: Vec<String>,
    pub digests: Vec<String>,
    pub label_keys: Vec<String>,
    pub created_at_epoch_seconds: Option<u64>,
    pub size_bytes: Option<u64>,
    pub shared_size_bytes: Option<u64>,
    pub container_count: Option<u64>,
}

pub struct ImageInventoryRow {
    pub observed_at_epoch_seconds: u64,
    pub images: Vec<ImageSummaryRow>,
}

fn image_summary_row(image: &susun::EngineImageSummary) -> ImageSummaryRow {
    ImageSummaryRow {
        id: image.id.as_str().to_owned(),
        references: image
            .references
            .iter()
            .map(|reference| reference.as_str().to_owned())
            .collect(),
        digests: image.digests.clone(),
        label_keys: image
            .label_keys
            .iter()
            .map(|key| key.as_str().to_owned())
            .collect(),
        created_at_epoch_seconds: image.created_at_epoch_seconds,
        size_bytes: image.size_bytes,
        shared_size_bytes: image.shared_size_bytes,
        container_count: image.container_count,
    }
}

/// Lists images across the whole engine. `capability` reflects the
/// provider's advertised support for engine-wide image inventory.
pub async fn image_inventory(
    engine: &DockerCompatibleEngine,
) -> Result<CapabilityResult<ImageInventoryRow>, String> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| error.to_string())?;
    let capability = enum_label(capabilities.supports_image_inventory);

    let data = match engine.image_inventory().await {
        Ok(inventory) => Some(ImageInventoryRow {
            observed_at_epoch_seconds: inventory.observed_at_epoch_seconds,
            images: inventory.images.iter().map(image_summary_row).collect(),
        }),
        Err(_) => None,
    };

    Ok(CapabilityResult { capability, data })
}

/// Reads one image from the engine-wide inventory.
pub async fn image_details(
    engine: &DockerCompatibleEngine,
    image_id: &str,
) -> Result<ImageSummaryRow, String> {
    let id =
        ImageId::new(image_id.to_owned()).map_err(|_| "image id must not be empty".to_owned())?;
    let image = engine
        .image_details(&id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(image_summary_row(&image))
}

/// Build-cache capability plus a non-destructive usage estimate, reusing the
/// same `cleanup_preview` primitive prune already relies on. Never prunes.
pub struct BuildCacheStatusRow {
    pub support: String,
    pub scope: Option<crate::susun_integration::CleanupScopeRow>,
}

pub async fn build_cache_status(
    engine: &DockerCompatibleEngine,
) -> Result<BuildCacheStatusRow, String> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| error.to_string())?;
    let support = enum_label(capabilities.supports_build_cache);

    let scope = if capabilities.supports_build_cache.is_supported() {
        crate::susun_integration::cleanup_preview(engine, &["build_cache".to_owned()], false)
            .await
            .ok()
            .and_then(|preview| preview.scopes.into_iter().next())
    } else {
        None
    };

    Ok(BuildCacheStatusRow { support, scope })
}

/// Registry capability flags only: whether the selected engine advertises
/// pull, push, and credential-backed registry operations. No live login
/// state, no configured-registry list, and no credential material — the SDK
/// has no such primitive, and none is added in this read-only foundation
/// slice.
pub struct RegistryCapabilityRow {
    pub supports_pull: String,
    pub supports_push: String,
    pub supports_auth: String,
}

pub async fn registry_capability(
    engine: &DockerCompatibleEngine,
) -> Result<RegistryCapabilityRow, String> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| error.to_string())?;
    Ok(RegistryCapabilityRow {
        supports_pull: enum_label(capabilities.supports_registry_pull),
        supports_push: enum_label(capabilities.supports_registry_push),
        supports_auth: enum_label(capabilities.supports_registry_auth),
    })
}

#[cfg(test)]
mod tests;
