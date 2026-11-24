//! Lock box commands (`services::lock_box`): Settings › Lock box. A key's
//! value comes in once, with `add_lock_box_key`, and never goes back out.

use crate::services::domain::lock_box::KeyRules;
use crate::services::lock_box::{self, LockBoxOverview, NewKey};

super::commands! {
    /// Every key by name and rules, and the OS unlock this computer has. Never a value.
    fn get_lock_box(state) -> LockBoxOverview => lock_box::overview(&state).await;

    /// Adds a key: its value straight into the OS keychain
    fn add_lock_box_key(state, key: NewKey) -> () => lock_box::add(&state, key).await;

    /// New rules for a key; its value stays as it is
    fn update_lock_box_key(state, name: String, rules: KeyRules) -> () => lock_box::update(&state, name, rules).await;

    /// Removes a key and its keychain entry
    fn delete_lock_box_key(state, name: String) -> () => lock_box::delete(&state, name).await;

    /// Unlocks a Locked key through the OS prompt; returns when it locks again (epoch ms)
    fn unlock_lock_box_key(state, name: String) -> f64 => lock_box::unlock_now(&state, name).await;

    /// Locks a key again now
    fn lock_lock_box_key(state, name: String) -> () => lock_box::lock_now(&state, name).await;
}
