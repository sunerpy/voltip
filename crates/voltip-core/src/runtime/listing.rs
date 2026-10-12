//! The models each provider lists (docs/dictation.md §3.3): the model selects offer what the
//! provider serves, not only the catalogue's presets. A list is fetched for the saved endpoint and
//! key when a model list opens (`CoreCommand::RefreshProviderModels`) and taken from a 测试连接
//! that used the saved values; it stays for the session and reaches the UI as
//! `ServiceStatus.listed`. A key change drops the provider's lists, and a list is not shown once
//! the endpoint it came from is no longer the saved one.

use std::collections::HashMap;

use super::{CoreEvent, Runtime};
use crate::CoreError;
use crate::engines::{EngineStatus, ProviderId, ServiceKind};
use crate::providers::{ProbeError, ProbeFailure, usable_models};

/// What a list request found.
#[derive(Debug)]
pub(super) struct Listed {
    pub(super) provider: ProviderId,
    pub(super) kind: ServiceKind,
    /// The endpoint it asked.
    pub(super) url: String,
    /// [`ModelLists::epoch`] when it left: a list fetched with an older key is dropped.
    pub(super) epoch: u64,
    /// A refresh (`RefreshProviderModels`), not a 测试连接.
    pub(super) refresh: bool,
    /// The models that can serve `kind`, or why there is no list.
    pub(super) models: Result<Vec<String>, ProbeError>,
}

/// The lists, by provider and service.
#[derive(Default)]
pub(super) struct ModelLists {
    /// The endpoint each list came from, and its models.
    lists: HashMap<(ProviderId, ServiceKind), (String, Vec<String>)>,
    /// Refreshes in flight, by the epoch they left in: one with an older key does not hold up a
    /// refresh with the new one.
    running: HashMap<(ProviderId, ServiceKind), u64>,
    /// Counts the key changes.
    pub(super) epoch: u64,
}

impl ModelLists {
    /// Fill `ServiceStatus.listed` of every service whose list came from the endpoint now saved
    /// (`ServiceStatus.base_url`).
    pub(super) fn fill(&self, status: &mut EngineStatus) {
        for provider in &mut status.providers {
            let id = provider.id;
            for (kind, service) in [(ServiceKind::Asr, provider.asr.as_mut()), (ServiceKind::Llm, provider.llm.as_mut())] {
                let Some(service) = service else { continue };
                if let Some((url, models)) = self.lists.get(&(id, kind))
                    && service.base_url.as_deref() == Some(url.as_str())
                {
                    service.listed.clone_from(models);
                }
            }
        }
    }

    /// `provider`'s key changed: its lists may be another account's.
    pub(super) fn forget(&mut self, provider: ProviderId) {
        self.lists.retain(|(p, _), _| *p != provider);
        self.epoch = self.epoch.wrapping_add(1);
    }
}

impl Runtime {
    /// `RefreshProviderModels`: list `provider`'s `kind` models with the saved endpoint and key on
    /// a task. Nothing to do for the built-in and on-device providers (the build and the model
    /// library name their models), without a saved key or address, on a shell without HTTP, or
    /// while the same refresh runs; the selects keep what they offer.
    pub(super) fn refresh_provider_models(&mut self, provider: ProviderId, kind: ServiceKind) -> Result<(), CoreError> {
        if matches!(provider, ProviderId::Builtin | ProviderId::Local) {
            return Ok(());
        }
        let Ok((url, key)) = self.probe_target(provider, kind, None, None) else { return Ok(()) };
        let Some(probe) = self.service_probe.clone() else { return Ok(()) };
        let (tx, epoch) = (self.listed_tx.clone(), self.model_lists.epoch);
        if self.model_lists.running.insert((provider, kind), epoch) == Some(epoch) {
            return Ok(());
        }
        tokio::spawn(async move {
            let models = probe.list_models(&url, key.as_deref()).await.map(|ids| usable_models(provider, kind, &url, ids));
            let _ = tx.send(Listed { provider, kind, url, epoch, refresh: true, models }).await;
        });
        Ok(())
    }

    /// A list arrived: kept when it was fetched with the current key from the endpoint now saved,
    /// and the UI told when it changed. A refused key drops the list; any other failure (offline,
    /// a timeout) keeps the last one.
    pub(super) fn on_listed(&mut self, listed: Listed) {
        let Listed { provider, kind, url, epoch, refresh, models } = listed;
        let slot = (provider, kind);
        if refresh && self.model_lists.running.get(&slot) == Some(&epoch) {
            self.model_lists.running.remove(&slot);
        }
        let saved = self.probe_target(provider, kind, None, None).ok().map(|(saved, _)| saved);
        if epoch != self.model_lists.epoch || saved.as_deref() != Some(url.as_str()) {
            return;
        }
        let changed = match models {
            Ok(models) => {
                let same = self.model_lists.lists.get(&slot).is_some_and(|(u, m)| *u == url && *m == models);
                if !same {
                    tracing::debug!(provider = provider.as_str(), ?kind, models = models.len(), "provider model list updated");
                    self.model_lists.lists.insert(slot, (url, models));
                }
                !same
            }
            Err(ProbeError { reason: ProbeFailure::Unauthorized, .. }) => self.model_lists.lists.remove(&slot).is_some(),
            Err(e) => {
                tracing::debug!(provider = provider.as_str(), ?kind, reason = ?e.reason, "provider model list not refreshed");
                false
            }
        };
        if changed {
            self.emit(CoreEvent::Engines(self.engine_status()));
        }
    }
}
