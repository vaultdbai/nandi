//! Own-device commands (Tauri adapter for `services::own_devices`):
//! Settings › Your devices. Desktop and local browser only: a paired phone
//! never reaches these, it speaks sealed envelopes through the relay.

use crate::services::own_devices::controls::AskLevel;
use crate::services::own_devices::{DeviceView, DevicesOverview, OwnDevices, PairingView};

super::commands! {
    /// The paired devices (`connected: false` without a relay)
    fn list_own_devices(state) -> DevicesOverview => OwnDevices::of(&state)?.overview().await;

    /// Opens a pairing: the QR code to show, once
    fn start_device_pairing(state, name: String) -> PairingView => OwnDevices::of(&state)?.start_pairing(&name).await;

    /// The fingerprints matched: activates the phone with its PIN
    fn confirm_own_device(state, device_id: String, pin: String) -> DeviceView => OwnDevices::of(&state)?.confirm(&device_id, &pin).await;

    fn revoke_own_device(state, device_id: String) -> () => OwnDevices::of(&state)?.revoke(&device_id).await;

    fn revoke_all_own_devices(state) -> () => OwnDevices::of(&state)?.revoke_all().await;

    /// How much a paired phone may ask: answer only, do with your yes, or off (works offline)
    fn set_own_device_ask_level(state, device_id: String, level: AskLevel) -> () => OwnDevices::of(&state)?.set_ask_level(&device_id, level).await;

    /// Unlocks a device the wrong PINs locked (works offline)
    fn unlock_own_device(state, device_id: String, pin: Option<String>) -> () => OwnDevices::of(&state)?.unlock(&device_id, pin).await;
}
