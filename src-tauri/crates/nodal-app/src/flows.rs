//! Cross-context flows (`App::delete_project`).

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
}
