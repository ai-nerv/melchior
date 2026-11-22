use super::{
    Header, Model, PATIENCE, Provider, Strategy, cache, cached_models, joined, ollama, ranked,
};
use crate::mind::provider::{endpoint::Auth, oauth, routing};
use futures_util::{StreamExt, stream};
use reqwest::{Url, header::HeaderMap};
use std::path::PathBuf;
use std::time::Duration;

const CONCURRENCY: usize = 4;
const TOTAL: Duration = Duration::from_secs(20);
pub(super) const MODELS: usize = 4096;

struct Job {
    index: usize,
    url: Url,
    headers: HeaderMap,
    path: Option<PathBuf>,
    popularity: bool,
    ollama: Option<String>,
}

pub(super) fn discover(providers: &mut [Provider], refresh: bool) -> Vec<String> {
    let mut jobs = Vec::new();
    let mut pending = std::collections::BTreeSet::new();
    for (index, provider) in providers.iter_mut().enumerate() {
        let strategy = Strategy::selected(provider);
        if strategy == Strategy::Bundled {
            continue;
        }
        let (key, usable) = credential(provider);
        let identity = serde_json::to_vec(&serde_json::json!({"id":provider.id,"base":provider.base_url,
            "api":provider.api,"auth":provider.auth,"headers":provider.headers,"origins":provider.credential_origins,
            "strategy":strategy,"avoid":provider.avoid})).ok();
        let path = identity.as_ref().and_then(|identity| {
            cache::path(
                "models",
                &[identity, key.as_deref().unwrap_or("").as_bytes()],
            )
        });
        let held = path
            .as_deref()
            .and_then(cache::read)
            .and_then(|held| Some((cached_models(&held.raw)?, held.fresh)));
        if let Some((models, fresh)) = held {
            provider.models = models
                .into_iter()
                .map(|model| joined(model, provider))
                .collect();
            if fresh && !refresh {
                continue;
            }
        }
        pending.insert(index);
        if !usable {
            continue;
        }
        if let Some(job) = job(index, provider, &strategy, key.as_deref(), path) {
            jobs.push(job);
        }
    }
    let received = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        Some(runtime.block_on(run(jobs, PATIENCE, TOTAL)))
    })
    .join()
    .ok()
    .flatten()
    .unwrap_or_default();
    for (index, models, path) in received {
        if let Some(path) = path
            && let Ok(raw) = serde_json::to_vec(&models)
        {
            let _ = cache::write(&path, &raw);
        }
        if let Some(provider) = providers.get_mut(index) {
            provider.models = models
                .into_iter()
                .map(|model| joined(model, provider))
                .collect();
            pending.remove(&index);
        }
    }
    pending
        .into_iter()
        .filter_map(|index| providers.get(index).map(|provider| provider.id.clone()))
        .collect()
}

fn credential(provider: &Provider) -> (Option<String>, bool) {
    match &provider.auth {
        Auth::None => (None, true),
        Auth::ApiKey { .. } => {
            let key = provider.resolve_key().ok().flatten();
            let usable = key.is_some() || Strategy::selected(provider) == Strategy::Openrouter;
            (key, usable)
        }
        Auth::OAuth { .. } | Auth::GitHubCopilot { .. } => {
            let token = oauth::Store::load()
                .ok()
                .and_then(|store| provider.saved_tokens(&store).cloned());
            let usable = token
                .as_ref()
                .is_some_and(|token| token.has_fresh_access(oauth::now()));
            (token.map(|token| token.access), usable)
        }
        Auth::GoogleAdc | Auth::AwsSigV4 => (None, false),
    }
}

fn job(
    index: usize,
    provider: &Provider,
    strategy: &Strategy,
    key: Option<&str>,
    path: Option<PathBuf>,
) -> Option<Job> {
    let base = provider.base_url.as_deref()?;
    let (suffix, auth, popularity, details) = match strategy {
        Strategy::Bundled => return None,
        Strategy::Compatible { path, auth } => (path.as_str(), *auth, false, None),
        Strategy::Openrouter if key.is_some() => ("models/user", Header::Bearer, false, None),
        Strategy::Openrouter => ("models?sort=most-popular", Header::Bearer, true, None),
        Strategy::Ollama => ("models", Header::Bearer, false, Some(ollama::root(base))),
    };
    let url = routing::provider_destination(
        provider,
        &format!("{}/{suffix}", base.trim_end_matches('/')),
    )
    .ok()?;
    let headers = routing::provider_headers(provider, auth.fields(key)).ok()?;
    Some(Job {
        index,
        url,
        headers,
        path,
        popularity,
        ollama: details,
    })
}

async fn run(
    jobs: Vec<Job>,
    patience: Duration,
    total: Duration,
) -> Vec<(usize, Vec<Model>, Option<PathBuf>)> {
    let Ok(client) = crate::mind::provider::http().timeout(patience).build() else {
        return Vec::new();
    };
    let deadline = tokio::time::Instant::now() + total;
    let mut work = stream::iter(jobs)
        .map(|job| {
            let client = client.clone();
            async move {
                if tokio::time::Instant::now() >= deadline {
                    return None;
                }
                let until = (tokio::time::Instant::now() + patience).min(deadline);
                let models = tokio::time::timeout_at(until, fetch(&client, &job))
                    .await
                    .ok()
                    .flatten();
                let models = match (models, &job.ollama) {
                    (Some(models), Some(root)) => {
                        Some(ollama::details(models, root, &client, &job.headers, until).await)
                    }
                    (models, _) => models,
                };
                models.map(|models| (job.index, models, job.path))
            }
        })
        .buffer_unordered(CONCURRENCY);
    let mut results = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Ok(Some(result)) = tokio::time::timeout_at(deadline, work.next()).await else {
            break;
        };
        if let Some(result) = result {
            results.push(result);
        }
    }
    results
}

async fn fetch(client: &crate::mind::provider::Http, job: &Job) -> Option<Vec<Model>> {
    let mut request = client.get(job.url.clone());
    for (name, value) in &job.headers {
        request = request.header(name.clone(), value.clone());
    }
    let response = request.send().await.ok()?.error_for_status().ok()?;
    let body = bounded(response).await?;
    let entries = body.get("data")?.as_array()?;
    if entries.len() > MODELS {
        return None;
    }
    let models = ranked(&body, job.popularity);
    if models.len() != entries.len() {
        return None;
    }
    if !super::identities(&models) {
        return None;
    }
    Some(models)
}

pub(super) async fn bounded(response: reqwest::Response) -> Option<serde_json::Value> {
    if response
        .content_length()
        .is_some_and(|size| size > cache::BYTES as u64)
    {
        return None;
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if chunk.len() > cache::BYTES.saturating_sub(body.len()) {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).ok()
}

#[cfg(test)]
mod tests;
