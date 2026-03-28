//! AlgoChat protocol implementation — send and receive encrypted messages
//! via Algorand transactions. Compatible with corvid-agent's TypeScript AlgoChat.

pub mod client;
pub mod listener;

pub use client::AlgoChatClient;
