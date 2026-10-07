//! Printer models and branch drawer configuration.

use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use super::device_repository::DeviceRepository;

/// The setting that says which printer model the branch uses.
pub const PRINTER_MODEL: &str = "printerModel";

/// The setting that says whether the branch has a cash drawer connected.
pub const HAS_DRAWER: &str = "hasDrawer";

/// The ESC/POS cash drawer pulse parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub struct DrawerPulse {
    pub pin: u8,
    pub on: u8,
    pub off: u8,
}

impl DrawerPulse {
    /// Builds the ESC/POS kick command bytes: `ESC @` then `ESC p pin on off`.
    pub fn kick_bytes(&self) -> Vec<u8> {
        vec![0x1B, 0x40, 0x1B, 0x70, self.pin, self.on, self.off]
    }
}

/// A printer model known to the system.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrinterModel {
    pub id: String,
    pub name: String,
    pub paper_mm: u16,
    pub drawer_pulse: Option<DrawerPulse>,
}

/// A registry of known printer models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterModels {
    models: Vec<PrinterModel>,
}

impl PrinterModels {
    /// Builds a registry from a list of models.
    ///
    /// Refuses an empty list, duplicate ids, pin > 1, on == 0, or off == 0.
    pub fn new(models: Vec<PrinterModel>) -> Result<Self, String> {
        if models.is_empty() {
            return Err("printer models list cannot be empty".to_string());
        }
        let mut seen_ids = std::collections::HashSet::new();
        for model in &models {
            if !seen_ids.insert(&model.id) {
                return Err(format!("duplicate printer model id: {}", model.id));
            }
            if let Some(pulse) = &model.drawer_pulse {
                if pulse.pin > 1 {
                    return Err(format!(
                        "printer model {} pulse pin must be 0 or 1, got {}",
                        model.id, pulse.pin
                    ));
                }
                if pulse.on == 0 {
                    return Err(format!(
                        "printer model {} pulse on duration must be >= 1, got 0",
                        model.id
                    ));
                }
                if pulse.off == 0 {
                    return Err(format!(
                        "printer model {} pulse off duration must be >= 1, got 0",
                        model.id
                    ));
                }
            }
        }
        Ok(Self { models })
    }

    /// The built-in registry containing only the NEOCAL NTP-8220BT entry.
    pub fn built_in() -> Self {
        Self::new(Self::built_in_models()).expect("built-in models are valid")
    }

    /// The default built-in printer model list.
    pub fn built_in_models() -> Vec<PrinterModel> {
        vec![PrinterModel {
            id: "NEOCAL_NTP_8220BT".to_string(),
            name: "NEOCAL NTP-8220BT".to_string(),
            paper_mm: 80,
            drawer_pulse: Some(DrawerPulse {
                pin: 0,
                on: 25,
                off: 250,
            }),
        }]
    }

    /// Looks up a printer model by its id.
    pub fn get(&self, id: &str) -> Option<&PrinterModel> {
        self.models.iter().find(|m| m.id == id)
    }

    /// The id of the first (default) printer model in the registry.
    pub fn default_id(&self) -> &str {
        &self.models[0].id
    }

    /// Returns a slice of all models in the registry.
    pub fn models(&self) -> &[PrinterModel] {
        &self.models
    }
}

impl Default for PrinterModels {
    fn default() -> Self {
        Self::built_in()
    }
}

/// The cash drawer configuration of a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawerSettings {
    pub model: String,
    pub has_drawer: Option<bool>,
}

impl DrawerSettings {
    /// Reads the drawer settings of a branch from the database.
    pub async fn read_in<R: DeviceRepository>(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        repo: &R,
        branch: &BranchId,
        models: &PrinterModels,
    ) -> AppResult<Self> {
        let model = match repo.setting_in(tx, branch, PRINTER_MODEL).await? {
            Some(m) => m,
            None => models.default_id().to_owned(),
        };

        let has_drawer = match repo.setting_in(tx, branch, HAS_DRAWER).await? {
            None => None,
            Some(ref s) if s == "0" => Some(false),
            Some(ref s) if s == "1" => Some(true),
            Some(other) => {
                return Err(pos_error::AppError::Internal(format!(
                    "{HAS_DRAWER}: invalid value {other}"
                )));
            }
        };

        Ok(Self { model, has_drawer })
    }
}
