// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Mode `catalog`: catalog requests to a provider, one after the other,
//! each with a fresh DCP token as `dsp-load` sends them, and the whole
//! answer read whatever its size.
//!
//! Settings (environment): `LOAD_URL` (the provider's catalog request
//! URL), `LOAD_AUDIENCE` (the provider's DID), `CATALOG_REQUESTS` (how
//! many, default 20), and the consumer's DCP settings as the connector
//! reads them (`PORTITOR_PARTICIPANT_ID`, `PORTITOR_STS_URL`, ...).
//!
//! It prints the answer's size, the number of datasets in it, and the
//! percentiles of the request (without the time to get the token).

use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use portitor_connector::auth::sts::{StsClient, StsProver};
use portitor_connector::auth::{AuthSettings, Prover as _};
use portitor_connector::http_client::{HttpsClient, https_client};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::time::Instant;

use crate::stats::{micros, print_percentiles};

pub(crate) async fn run() -> Result<(), String> {
    let url = crate::required("LOAD_URL")?;
    let audience = crate::required("LOAD_AUDIENCE")?;
    let requests = crate::number("CATALOG_REQUESTS", 20)?;
    let participant = crate::required("PORTITOR_PARTICIPANT_ID")?;
    let settings =
        AuthSettings::from_env(&participant, true)?.ok_or("PORTITOR_STS_URL is required")?;
    let prover = StsProver::new(Arc::new(StsClient::new(&settings)?), settings.scopes);
    let (client, _) = https_client(true)?;
    let mut times = Vec::new();
    let mut answer = Bytes::new();
    for _ in 0..requests {
        // a DCP token may be used once: a fresh one for each request
        let token = prover.authorization(&audience, &[]).await?;
        let start = Instant::now();
        answer = catalog_request(&client, &url, &token).await?;
        times.push(micros(start.elapsed()));
    }
    let catalog: Value = serde_json::from_slice(&answer).map_err(|e| e.to_string())?;
    let datasets = catalog
        .get("dataset")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    println!("requests={requests}");
    println!("answer_bytes={}", answer.len());
    println!("datasets={datasets}");
    print_percentiles("request", times);
    Ok(())
}

/// Sends one catalog request and returns the answer's body.
async fn catalog_request(client: &HttpsClient, url: &str, token: &str) -> Result<Bytes, String> {
    let message = json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@type": "CatalogRequestMessage",
        "filter": []
    });
    let request = hyper::Request::post(url)
        .header("authorization", token)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(message.to_string())))
        .map_err(|e| e.to_string())?;
    let response = client.request(request).await.map_err(|e| e.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let body = response.into_body().collect().await;
    Ok(body.map_err(|e| e.to_string())?.to_bytes())
}
