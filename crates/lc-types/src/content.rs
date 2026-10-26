use serde::{Deserialize, Serialize};
use specta::Type;

use crate::version::LoaderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ContentSourceKind {
    Modrinth,
    CurseForge,
    /// File added manually by the user.
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ContentKind {
    Mod,
    ResourcePack,
    Shader,
    DataPack,
    Modpack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    Relevance,
    Downloads,
    Updated,
    Newest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub source: ContentSourceKind,
    pub kind: ContentKind,
    pub text: String,
    pub mc: Option<String>,
    pub loader: Option<LoaderKind>,
    pub categories: Vec<String>,
    pub sort: SortBy,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub source: ContentSourceKind,
    /// Source-specific project id (Modrinth id / CurseForge numeric id as string).
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub kind: ContentKind,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub items: Vec<ProjectSummary>,
    pub total: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DependencyKind {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub project_id: String,
    pub version_id: Option<String>,
    pub kind: DependencyKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContentVersion {
    pub source: ContentSourceKind,
    pub project_id: String,
    pub version_id: String,
    pub name: String,
    pub version_number: String,
    pub mc_versions: Vec<String>,
    pub loaders: Vec<LoaderKind>,
    /// RFC 3339.
    pub published: String,
    pub file_name: String,
    pub size: u64,
    pub sha1: Option<String>,
    pub dependencies: Vec<Dependency>,
    /// CurseForge `allowModDistribution=false`.
    pub manual_download: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContentFilter {
    pub mc: Option<String>,
    pub loader: Option<LoaderKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContentVersionRef {
    pub source: ContentSourceKind,
    pub project_id: String,
    /// `None` = latest compatible.
    pub version_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub project_id: String,
    pub with_project_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManualDownload {
    pub project_id: String,
    pub file_name: String,
    /// Page the user must open in a browser.
    pub url: String,
}

/// Result of dependency resolution; confirmed by the user, then installed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResolvePlan {
    pub install: Vec<ContentVersion>,
    pub already_installed: Vec<String>,
    pub conflicts: Vec<Conflict>,
    pub manual: Vec<ManualDownload>,
}

/// File present in an instance (`mods/`, `resourcepacks/`, `shaderpacks/`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledContent {
    pub file_name: String,
    pub kind: ContentKind,
    pub enabled: bool,
    pub source: ContentSourceKind,
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub title: Option<String>,
    pub version_number: Option<String>,
    pub update_available: bool,
}

/// What is wrong with a dependency declared inside a local mod jar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LocalDepIssueKind {
    /// Required mod is not installed (or disabled).
    Missing,
    /// Installed, but its version doesn't match the requirement.
    WrongVersion,
    /// Declared as incompatible with this one and installed.
    Conflict,
    /// The jar is built for another loader or Minecraft version.
    WrongLoader,
}

/// A problem found by `ContentService::check_local` in a local mod's metadata
/// (`fabric.mod.json`, `quilt.mod.json`, `META-INF/(neoforge.)mods.toml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalDepIssue {
    /// Local file that declares the dependency.
    pub file_name: String,
    pub kind: LocalDepIssueKind,
    /// Mod id from the metadata (`fabric-api`, `geckolib`); for `WrongLoader`
    /// the loader/version the jar expects.
    pub mod_id: String,
    /// Requirement as written in the metadata.
    pub version_req: Option<String>,
    /// A Modrinth version that fixes `Missing`/`WrongVersion`, if found; pass it
    /// to `ContentService::resolve`.
    pub suggestion: Option<ContentVersionRef>,
    pub suggestion_title: Option<String>,
}
