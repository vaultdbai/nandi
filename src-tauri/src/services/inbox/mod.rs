//! Every Inbox item is an Ask chat with the Chief of Staff. Opening an item
//! opens its thread (`inbox:<kind>:<id>`, [`thread`]): the person asks
//! about it, answers in their own words, decides with conditions or hands
//! it on. The Chief of Staff gets the item as untrusted `<task-data>`
//! and, in that thread only, the decision tools ([`tool`]): each call waits
//! for the person to confirm it on its approval card, then calls the same
//! service function as the item's button. Every decision, from a button
//! or a chat, is written to the gold copy ([`audit`]).
//!
//! An intake item keeps its own thread (`intake:<id>`,
//! `services::intake::thread`), with the same decision tools.

pub mod audit;
pub mod item;
pub mod thread;
pub mod tool;
