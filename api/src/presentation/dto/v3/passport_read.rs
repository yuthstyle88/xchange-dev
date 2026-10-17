//! The desk's passport read answer (design `docs/design/recognize-passport-read.md` §4.3), generated
//! to `web/src/shared/api/generated/passport_read.ts`. Field names are the desk's `PassportFields`.

use std::path::Path;

use serde::Serialize;
use ts_rs::TS;

use crate::application::passport_read::{AiLocation, AiProvider, PassportRead, VisualZone};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "passport_read.ts")]
pub struct PassportReadAnswer {
    /// `null` when Recognize read the image but refused what it read (not an error).
    pub fields: Option<PassportReadFields>,
    /// 0–5; 0 when `fields` is null.
    pub score: u8,
    /// null when `fields` is null.
    pub visual_zone: Option<VisualZoneDto>,
    pub auto_acceptable: bool,
    pub ai: PassportReadAi,
}

/// Each value is a string or null, never "".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "passport_read.ts")]
pub struct PassportReadFields {
    pub name: Option<String>,
    pub passport_no: Option<String>,
    pub nationality: Option<String>,
    /// `YYYY-MM-DD`.
    pub birth_date: Option<String>,
    /// `YYYY-MM-DD`.
    pub passport_expiry: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "passport_read.ts")]
pub enum VisualZoneDto {
    Same,
    Differs,
    Unchecked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "passport_read.ts")]
pub enum AiProviderDto {
    Gemini,
    Ollama,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "passport_read.ts")]
pub enum AiLocationDto {
    Cloud,
    Local,
}

/// Which AI produced the answer, as Recognize echoed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "passport_read.ts")]
pub struct PassportReadAi {
    pub provider: AiProviderDto,
    pub model: String,
    pub location: AiLocationDto,
}

impl PassportReadAnswer {
    /// Maps one Recognize answer (design §4.3 table).
    pub fn of(read: &PassportRead) -> Self {
        let (fields, score, visual_zone) = match &read.result {
            Some(r) => (
                Some(PassportReadFields {
                    name: r.name.clone(),
                    passport_no: r.passport_no.clone(),
                    nationality: r.nationality.clone(),
                    birth_date: r.birth_date.clone(),
                    passport_expiry: r.expiry_date.clone(),
                }),
                r.score,
                Some(match r.visual_zone {
                    VisualZone::Same => VisualZoneDto::Same,
                    VisualZone::Differs => VisualZoneDto::Differs,
                    VisualZone::Unchecked => VisualZoneDto::Unchecked,
                }),
            ),
            None => (None, 0, None),
        };

        Self {
            fields,
            score,
            visual_zone,
            auto_acceptable: read.auto_acceptable,
            ai: PassportReadAi {
                provider: match read.provider {
                    AiProvider::Gemini => AiProviderDto::Gemini,
                    AiProvider::Ollama => AiProviderDto::Ollama,
                },
                model: read.model.clone(),
                location: match read.location {
                    AiLocation::Cloud => AiLocationDto::Cloud,
                    AiLocation::Local => AiLocationDto::Local,
                },
            },
        }
    }
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PassportReadAnswer::export_all(&cfg)?;
    Ok(())
}
