//! Проверка доступности хостов: короткая попытка TCP-соединения на порт подключения.
//! Без авторизации и передачи данных — соединение сразу закрывается.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

#[derive(Deserialize)]
pub struct Target {
    pub id: String,
    pub host: String,
    pub port: u16,
}

#[derive(Serialize)]
pub struct Reach {
    pub id: String,
    pub up: bool,
}

/// Проверить хосты параллельно (не больше `parallel` одновременно).
pub async fn probe(targets: Vec<Target>, timeout_ms: u64, parallel: usize) -> Vec<Reach> {
    let sem = Arc::new(Semaphore::new(parallel.clamp(1, 64)));
    let timeout = Duration::from_millis(timeout_ms.clamp(200, 10_000));
    let mut tasks = Vec::with_capacity(targets.len());
    for t in targets {
        let sem = sem.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await;
            let addr = format!("{}:{}", t.host.trim_start_matches('[').trim_end_matches(']'), t.port);
            let up = matches!(
                tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await,
                Ok(Ok(_))
            );
            Reach { id: t.id, up }
        }));
    }
    let mut out = Vec::with_capacity(tasks.len());
    for t in tasks {
        if let Ok(r) = t.await {
            out.push(r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_local() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let r = probe(
            vec![
                Target { id: "a".into(), host: "127.0.0.1".into(), port },
                Target { id: "b".into(), host: "127.0.0.1".into(), port: 1 },
            ],
            800,
            4,
        )
        .await;
        assert!(r.iter().find(|x| x.id == "a").unwrap().up);
        assert!(!r.iter().find(|x| x.id == "b").unwrap().up);
    }
}
