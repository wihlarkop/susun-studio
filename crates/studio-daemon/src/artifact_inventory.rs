//! Engine-wide artifact inventory (images, containers, build cache, and
//! registry capability) sourced entirely from the public Susun facade.
//! Everything here is read-only: nothing pulls, pushes, builds, tags, or
//! prunes. Mutating artifact workflows are Phase 15b+.

use susun::{
    ContainerEngine, ContainerId, DockerCompatibleEngine, EngineError, ImageId, ProjectName,
};
use turso::Database;

use crate::susun_integration::enum_label;

/// Typed failure for artifact-inventory operations, so a genuine provider
/// fault (Docker/Podman itself failed — a 502) stays distinguishable from a
/// fault in Studio's own database (a 500). Flattening both into one opaque
/// string, as this module used to, loses that distinction for the caller.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("{0}")]
    Provider(String),
    #[error("database error: {0}")]
    Database(#[from] turso::Error),
}

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
/// means the platform default engine (no profile selected or bound). A
/// database fault propagates as `Err` rather than silently collapsing into
/// "no attribution" — the two must stay distinguishable, since the latter is
/// a normal state but the former is a daemon fault worth surfacing as one.
pub async fn runtime_context(
    db: &Database,
    profile_id: Option<&str>,
) -> Result<RuntimeContextRow, ArtifactError> {
    let profile = match profile_id {
        Some(id) => crate::runtime::list_all_profiles(db)
            .await?
            .into_iter()
            .find(|profile| profile.id == id),
        None => None,
    };
    Ok(match profile {
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
    })
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
/// when the provider does not support the operation — never an HTTP error,
/// so unsupported providers surface as an explicit capability state rather
/// than a generic failure. A provider that *does* support the operation but
/// fails the call is a genuine fault and propagates as `Err`, never folded
/// into this same "no data" shape.
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
/// `data` is `None` only when the provider does not support this operation;
/// a provider that *does* support it but fails the call is a real failure
/// and propagates as `Err`, never a silent empty-looking success.
pub async fn container_inventory(
    db: &Database,
    engine: &DockerCompatibleEngine,
) -> Result<CapabilityResult<ContainerInventoryRow>, ArtifactError> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| ArtifactError::Provider(error.to_string()))?;
    let capability = enum_label(capabilities.supports_container_inventory);

    if !capabilities.supports_container_inventory.is_supported() {
        return Ok(CapabilityResult {
            capability,
            data: None,
        });
    }

    let inventory = engine
        .container_inventory()
        .await
        .map_err(|error| ArtifactError::Provider(error.to_string()))?;
    let known = known_projects(db).await?;
    let data = Some(ContainerInventoryRow {
        observed_at_epoch_seconds: inventory.observed_at_epoch_seconds,
        containers: inventory
            .containers
            .iter()
            .map(|container| container_summary_row(container, &known))
            .collect(),
    });

    Ok(CapabilityResult { capability, data })
}

/// Outcome of looking up one artifact by id from the engine-wide inventory.
/// Distinguishes "the provider doesn't support this at all" (an explicit
/// capability state, never an error) from "the provider supports it but no
/// such id exists" (a genuine 404) from an actual provider failure
/// (propagated as `Err`, mapped to a 502 by the caller). `Found` carries the
/// real support level (`supported` vs `supported_subset`) rather than
/// assuming full support — a caller that only reads partial data still
/// deserves to know that, the same way the list endpoints already do.
pub enum DetailLookup<T> {
    Found { capability: String, value: T },
    Unsupported { capability: String },
    NotFound,
}

/// Reads one container from the engine-wide inventory.
pub async fn container_details(
    db: &Database,
    engine: &DockerCompatibleEngine,
    container_id: &str,
) -> Result<DetailLookup<ContainerSummaryRow>, ArtifactError> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| ArtifactError::Provider(error.to_string()))?;
    let capability = enum_label(capabilities.supports_container_inventory);
    if !capabilities.supports_container_inventory.is_supported() {
        return Ok(DetailLookup::Unsupported { capability });
    }

    let id = ContainerId::new(container_id.to_owned())
        .map_err(|_| ArtifactError::Provider("container id must not be empty".to_owned()))?;
    match engine.container_details(&id).await {
        Ok(container) => {
            let known = known_projects(db).await?;
            Ok(DetailLookup::Found {
                capability,
                value: container_summary_row(&container, &known),
            })
        }
        Err(EngineError::NotFound { .. }) => Ok(DetailLookup::NotFound),
        Err(error) => Err(ArtifactError::Provider(error.to_string())),
    }
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
/// provider's advertised support for engine-wide image inventory. `data` is
/// `None` only when the provider does not support this operation; a real
/// call failure on a supported provider propagates as `Err`.
pub async fn image_inventory(
    engine: &DockerCompatibleEngine,
) -> Result<CapabilityResult<ImageInventoryRow>, ArtifactError> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| ArtifactError::Provider(error.to_string()))?;
    let capability = enum_label(capabilities.supports_image_inventory);

    if !capabilities.supports_image_inventory.is_supported() {
        return Ok(CapabilityResult {
            capability,
            data: None,
        });
    }

    let inventory = engine
        .image_inventory()
        .await
        .map_err(|error| ArtifactError::Provider(error.to_string()))?;
    let data = Some(ImageInventoryRow {
        observed_at_epoch_seconds: inventory.observed_at_epoch_seconds,
        images: inventory.images.iter().map(image_summary_row).collect(),
    });

    Ok(CapabilityResult { capability, data })
}

/// Reads one image from the engine-wide inventory.
pub async fn image_details(
    engine: &DockerCompatibleEngine,
    image_id: &str,
) -> Result<DetailLookup<ImageSummaryRow>, ArtifactError> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| ArtifactError::Provider(error.to_string()))?;
    let capability = enum_label(capabilities.supports_image_inventory);
    if !capabilities.supports_image_inventory.is_supported() {
        return Ok(DetailLookup::Unsupported { capability });
    }

    let id = ImageId::new(image_id.to_owned())
        .map_err(|_| ArtifactError::Provider("image id must not be empty".to_owned()))?;
    match engine.image_details(&id).await {
        Ok(image) => Ok(DetailLookup::Found {
            capability,
            value: image_summary_row(&image),
        }),
        Err(EngineError::NotFound { .. }) => Ok(DetailLookup::NotFound),
        Err(error) => Err(ArtifactError::Provider(error.to_string())),
    }
}

/// Build-cache capability plus a non-destructive usage estimate, reusing the
/// same `cleanup_preview` primitive prune already relies on. Never prunes.
pub struct BuildCacheStatusRow {
    pub support: String,
    pub scope: Option<crate::susun_integration::CleanupScopeRow>,
}

pub async fn build_cache_status(
    engine: &DockerCompatibleEngine,
) -> Result<BuildCacheStatusRow, ArtifactError> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| ArtifactError::Provider(error.to_string()))?;
    let support = enum_label(capabilities.supports_build_cache);

    let scope = if capabilities.supports_build_cache.is_supported() {
        let preview =
            crate::susun_integration::cleanup_preview(engine, &["build_cache".to_owned()], false)
                .await
                .map_err(ArtifactError::Provider)?;
        preview.scopes.into_iter().next()
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
) -> Result<RegistryCapabilityRow, ArtifactError> {
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| ArtifactError::Provider(error.to_string()))?;
    Ok(RegistryCapabilityRow {
        supports_pull: enum_label(capabilities.supports_registry_pull),
        supports_push: enum_label(capabilities.supports_registry_push),
        supports_auth: enum_label(capabilities.supports_registry_auth),
    })
}

#[cfg(test)]
mod tests;
