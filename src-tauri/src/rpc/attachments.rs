//! Attachment commands (`services::attachments`). Picking files is
//! desktop-only: `desktop::attachments`.

use crate::services::attachments;

super::commands! {
    fn discard_attachment(id: String) -> () => attachments::discard_attachment(id).await;
}
