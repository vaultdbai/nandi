//! Trigger commands (`services::triggers`). The webhook and device receivers
//! stay hand-written routes in `http/triggers.rs`: they authenticate with the
//! trigger's own secret.

use crate::services::triggers;
use crate::services::triggers::calendar::CalendarPreview;
use crate::services::triggers::cron::CronPreview;
use crate::services::triggers::schema::{SavedTrigger, Trigger, TriggerFire, TriggerInput};
use serde_json::Value;

super::commands! {
    fn list_triggers(state) -> Vec<Trigger> => triggers::list_triggers(&state).await;

    fn get_trigger(state, id: String) -> Trigger => triggers::get_trigger(&state, id).await;

    fn create_trigger(state, input: TriggerInput) -> SavedTrigger => triggers::create_trigger(&state, input).await;

    fn update_trigger(state, id: String, input: TriggerInput) -> SavedTrigger => triggers::update_trigger(&state, id, input).await;

    fn delete_trigger(state, id: String) -> () => triggers::delete_trigger(&state, id).await;

    fn set_trigger_enabled(state, id: String, enabled: bool) -> () => triggers::set_trigger_enabled(&state, id, enabled).await;

    fn rotate_trigger_secret(state, id: String) -> SavedTrigger => triggers::rotate_trigger_secret(&state, id).await;

    fn test_fire_trigger(state, id: String, payload: Option<Value>) -> TriggerFire => triggers::test_fire_trigger(&state, id, payload).await;

    fn list_trigger_fires(state, trigger_id: String, limit: Option<u32>) -> Vec<TriggerFire> => triggers::list_trigger_fires(&state, trigger_id, limit).await;

    /// Where webhook and device requests go on this machine, e.g.
    /// `http://127.0.0.1:17463` (`None` until the web server is listening).
    fn get_web_origin(state) -> Option<String> => Ok(state.http_port().map(|port| format!("http://127.0.0.1:{port}")));

    /// Whether a calendar is valid and its next 5 runs with who is on duty
    fn preview_calendar(config: Value) -> CalendarPreview => Ok(triggers::calendar::preview(&config, chrono::Local::now(), triggers::calendar::PREVIEW_COUNT));

    /// Whether a cron schedule is valid and its next 3 runs (the scheduler's
    /// own parser, so they match)
    fn preview_cron(schedule: String) -> CronPreview => Ok(triggers::cron::preview(&schedule, chrono::Local::now(), triggers::cron::PREVIEW_COUNT));
}
