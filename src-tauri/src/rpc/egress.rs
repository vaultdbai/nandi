//! Egress commands: proxy status and AWS credentials.

use crate::services::egress::{self, EgressStatus};

super::commands! {
    fn get_egress_status(state) -> EgressStatus => egress::get_egress_status(&state).await;

    /// Stores the AWS key in the keychain; empty values remove it.
    fn set_aws_credentials(state, access_key: String, secret: String) -> () => egress::set_aws_credentials(&state, access_key, secret).await;

    fn set_aws_region(state, region: String) -> () => egress::set_aws_region(&state, region).await;
}
