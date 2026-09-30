//! "Import data from a previous version…": Tauri command, thin over `System::import_legacy`.
//! The import logic (copy, then import inside a transaction) lives in `nodal_store::legacy`.

use std::sync::Arc;

use tauri::State;

use nodal_app::App;

use crate::commands::CommandError;

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn import_legacy_data(
    db: State<'_, Arc<App>>,
    folder: String,
) -> Result<nodal_store::legacy::LegacyImportReport, CommandError> {
    db.system.import_legacy(folder).await.map_err(CommandError::from)
}
