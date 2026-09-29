//! Cross-context flows (`App::delete_project`, `App::set_settings`): filled in wave 4, once
//! `board`, `execution`, `chats` and `sources` have real bodies to call into.

use std::sync::Arc;

use nodal_domain::model::Settings;

use crate::core::AppError;
use crate::App;

impl App {
    /// Today's `commands::board::delete_project`: stops the project's chats right after
    /// `check_id`, then deletes it.
    pub async fn delete_project(&self, id: String) -> Result<(), AppError> {
        self.board
            .delete_project(id, |p| self.chats.stop_project(p))
            .await
    }

    /// Today's `commands::board::set_settings`: saves the settings, then kicks the queue since
    /// more concurrency may free up room for queued runs.
    pub async fn set_settings(self: &Arc<Self>, settings: Settings) -> Result<Settings, AppError> {
        let s = self.board.set_settings(settings).await?;
        self.kick();
        Ok(s)
    }
}
