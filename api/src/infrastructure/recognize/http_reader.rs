//! `RecognizeHttpReader`: one `POST <base>/v1/read` per read (design
//! `docs/design/recognize-passport-read.md` §4.4, `ai-reader.md` §4).

use std::time::Instant;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::json;

use super::settings::RecognizeSettings;
use crate::application::passport_read::{
    AiLocation, AiProvider, PassportImage, PassportRead, PassportReader, PassportResult, ReadError,
    VisualZone,
};

pub struct RecognizeHttpReader {
    client: reqwest::Client,
    settings: RecognizeSettings,
}

impl RecognizeHttpReader {
    pub fn new(settings: &RecognizeSettings) -> Self {
        let client =
            crate::infrastructure::http_client::build_client(settings.timeout, "xchange-recognize");
        Self {
            client,
            settings: settings.clone(),
        }
    }
}

#[derive(Deserialize)]
struct WireReadAnswer {
    ai: WireAi,
    result: Option<WirePassportResult>,
    auto_acceptable: bool,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireAi {
    provider: String,
    model: String,
    location: String,
}

#[derive(Deserialize)]
struct WirePassportResult {
    name: Option<String>,
    passport_no: Option<String>,
    nationality: Option<String>,
    birth_date: Option<String>,
    expiry_date: Option<String>,
    score: u8,
    visual_zone: String,
}

#[derive(Deserialize)]
struct WireUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct WireErrorBody {
    error: Option<String>,
}

fn outcome_code(err: &ReadError) -> &'static str {
    match err {
        ReadError::NotConfigured => "passportReaderNotConfigured",
        ReadError::Busy { .. } => "passportReaderBusy",
        ReadError::Failed => "passportReaderFailed",
        ReadError::Timeout => "passportReaderTimeout",
        ReadError::ImageInvalid => "imageInvalid",
        ReadError::ImageTooLarge => "imageTooLarge",
        ReadError::PdfNotAccepted => "pdfNotAccepted",
        ReadError::Internal => "serverError",
    }
}

#[async_trait]
impl PassportReader for RecognizeHttpReader {
    async fn read(&self, image: &PassportImage) -> Result<PassportRead, ReadError> {
        let started = Instant::now();
        let base = self
            .settings
            .base_url
            .strip_suffix('/')
            .unwrap_or(&self.settings.base_url);
        let url = format!("{base}/v1/read");

        let ai = match &self.settings.model {
            Some(model) => json!({
                "provider": self.settings.provider.as_str(),
                "model": model,
            }),
            None => json!({
                "provider": self.settings.provider.as_str(),
            }),
        };

        let body = json!({
            "document": "passport",
            "ai": ai,
            "image": {
                "mime": image.mime().as_str(),
                "data": STANDARD.encode(image.bytes()),
            },
        });

        let response = match self
            .client
            .post(&url)
            .bearer_auth(self.settings.token())
            .json(&body)
            .send()
            .await
        {
            Ok(res) => res,
            Err(err) => {
                let mapped = if err.is_timeout() {
                    ReadError::Timeout
                } else {
                    ReadError::Failed
                };
                tracing::info!(
                    provider = self.settings.provider.as_str(),
                    model = self.settings.model.as_deref().unwrap_or(""),
                    outcome = outcome_code(&mapped),
                    elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    "passport read finished"
                );
                return Err(mapped);
            }
        };

        let status = response.status().as_u16();
        let retry_after_secs = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u32>().ok())
            .filter(|&n| (1..=60).contains(&n))
            .unwrap_or(5);

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(err) => {
                let mapped = if err.is_timeout() {
                    ReadError::Timeout
                } else {
                    ReadError::Failed
                };
                tracing::info!(
                    provider = self.settings.provider.as_str(),
                    model = self.settings.model.as_deref().unwrap_or(""),
                    outcome = outcome_code(&mapped),
                    elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    "passport read finished"
                );
                return Err(mapped);
            }
        };

        if status == 200 {
            let parsed = match serde_json::from_slice::<WireReadAnswer>(&bytes) {
                Ok(w) => map_wire_answer(w),
                Err(_) => Err(ReadError::Failed),
            };
            match parsed {
                Ok((read, usage)) => {
                    let location_str = match read.location {
                        AiLocation::Cloud => "cloud",
                        AiLocation::Local => "local",
                    };
                    let score = read.result.as_ref().map_or(0, |r| r.score);
                    let input_tokens = usage.as_ref().and_then(|u| u.input_tokens);
                    let output_tokens = usage.as_ref().and_then(|u| u.output_tokens);
                    tracing::info!(
                        provider = read.provider.as_str(),
                        model = %read.model,
                        location = location_str,
                        outcome = "ok",
                        auto_acceptable = read.auto_acceptable,
                        score = score,
                        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        input_tokens = ?input_tokens,
                        output_tokens = ?output_tokens,
                        "passport read finished"
                    );
                    Ok(read)
                }
                Err(mapped) => {
                    tracing::info!(
                        provider = self.settings.provider.as_str(),
                        model = self.settings.model.as_deref().unwrap_or(""),
                        outcome = outcome_code(&mapped),
                        elapsed_ms =
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        "passport read finished"
                    );
                    Err(mapped)
                }
            }
        } else {
            let recognize_code = serde_json::from_slice::<WireErrorBody>(&bytes)
                .ok()
                .and_then(|b| b.error);
            let code_str = recognize_code.as_deref().unwrap_or("");
            let mapped = match (status, code_str) {
                (400, "IMAGE_INVALID") => ReadError::ImageInvalid,
                (413, _) => ReadError::ImageTooLarge,
                (415, _) => ReadError::PdfNotAccepted,
                (401, _) => {
                    tracing::error!(
                        recognize_error = code_str,
                        "Recognize rejected our authentication"
                    );
                    ReadError::NotConfigured
                }
                (400, "AI_UNKNOWN" | "AI_MODEL_NOT_ALLOWED") => {
                    tracing::error!(
                        recognize_error = code_str,
                        "Recognize rejected our AI provider or model setting"
                    );
                    ReadError::NotConfigured
                }
                (503, "AI_NOT_CONFIGURED") | (501, _) => ReadError::NotConfigured,
                (429, _) => ReadError::Busy { retry_after_secs },
                (504, _) => ReadError::Timeout,
                (500..=599, _) => ReadError::Failed,
                (400..=499, _) => ReadError::Internal,
                _ => ReadError::Failed,
            };
            tracing::info!(
                provider = self.settings.provider.as_str(),
                model = self.settings.model.as_deref().unwrap_or(""),
                outcome = outcome_code(&mapped),
                elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "passport read finished"
            );
            Err(mapped)
        }
    }
}

fn map_wire_answer(wire: WireReadAnswer) -> Result<(PassportRead, Option<WireUsage>), ReadError> {
    let provider = match wire.ai.provider.as_str() {
        "gemini" => AiProvider::Gemini,
        "ollama" => AiProvider::Ollama,
        _ => return Err(ReadError::Failed),
    };
    let location = match wire.ai.location.as_str() {
        "cloud" => AiLocation::Cloud,
        "local" => AiLocation::Local,
        _ => return Err(ReadError::Failed),
    };
    let result = match wire.result {
        None => None,
        Some(r) => {
            let visual_zone = match r.visual_zone.as_str() {
                "same" => VisualZone::Same,
                "differs" => VisualZone::Differs,
                "unchecked" => VisualZone::Unchecked,
                _ => return Err(ReadError::Failed),
            };
            Some(PassportResult {
                name: r.name,
                passport_no: r.passport_no,
                nationality: r.nationality,
                birth_date: r.birth_date,
                expiry_date: r.expiry_date,
                score: r.score,
                visual_zone,
            })
        }
    };
    Ok((
        PassportRead {
            provider,
            model: wire.ai.model,
            location,
            result,
            auto_acceptable: wire.auto_acceptable,
        },
        wire.usage,
    ))
}
