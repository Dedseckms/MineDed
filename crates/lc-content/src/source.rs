//! Internal extension of `ContentSource` used by the service, resolver and packs.

use std::collections::HashMap;

use async_trait::async_trait;
use lc_types::content::{ContentFilter, ContentKind, ContentVersion};
use lc_types::version::LoaderKind;
use lc_types::AppResult;

use crate::contract::ContentSource;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectInfo {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub kind: ContentKind,
    /// Human page (manual downloads, links).
    pub page_url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RemoteFile {
    /// `None` = may not be downloaded automatically (CurseForge opt-out).
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    /// Page where the user downloads the file by hand.
    pub manual_url: String,
}

/// Downloadable modpack archive of one catalogue version.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PackFile {
    pub version_id: String,
    pub file_name: String,
    pub url: String,
    /// Always present: packs without a SHA-1 are rejected.
    pub sha1: String,
    pub sha512: Option<String>,
    pub size: u64,
    /// Project title (instance name).
    pub title: String,
    pub icon_url: Option<String>,
}

/// Instance the content is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub mc: String,
    pub loader: LoaderKind,
}

impl Target {
    /// Filter for `kind`: only mods (and modpacks) are filtered by loader.
    pub fn filter(&self, kind: ContentKind) -> ContentFilter {
        let by_loader = matches!(kind, ContentKind::Mod | ContentKind::Modpack)
            && self.loader != LoaderKind::Vanilla;
        ContentFilter {
            mc: Some(self.mc.clone()),
            loader: by_loader.then_some(self.loader),
        }
    }
}

/// Installed file to check for updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateItem {
    pub project_id: String,
    pub version_id: String,
    pub sha1: Option<String>,
    pub kind: ContentKind,
}

#[async_trait]
pub(crate) trait Catalog: ContentSource {
    async fn project(&self, id: &str) -> AppResult<ProjectInfo>;

    /// Versions with a "release" flag, newest first.
    async fn versions_ranked(
        &self,
        project_id: &str,
        filter: &ContentFilter,
    ) -> AppResult<Vec<(ContentVersion, bool)>>;

    async fn file(&self, v: &ContentVersion) -> AppResult<RemoteFile>;

    /// Newest compatible version, releases preferred.
    async fn latest(&self, project_id: &str, target: &Target) -> AppResult<Option<ContentVersion>> {
        let kind = self.project(project_id).await?.kind;
        if kind == ContentKind::Mod && target.loader == LoaderKind::Vanilla {
            return Ok(None);
        }
        let list = self
            .versions_ranked(project_id, &target.filter(kind))
            .await?;
        let list: Vec<_> = list
            .into_iter()
            .filter(|(v, _)| {
                kind != ContentKind::Mod
                    || crate::util::loader_compatible(&v.loaders, target.loader)
            })
            .collect();
        Ok(list
            .iter()
            .find(|(_, release)| *release)
            .or_else(|| list.first())
            .map(|(v, _)| v.clone()))
    }

    /// Newer versions for installed files, keyed by project id.
    async fn updates(
        &self,
        items: &[UpdateItem],
        target: &Target,
        fresh: bool,
    ) -> AppResult<HashMap<String, ContentVersion>>;
}
