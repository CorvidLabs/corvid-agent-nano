//! X25519 + ChaCha20-Poly1305 encryption for AlgoChat messages.
//! Compatible with corvid-agent's TypeScript implementation.

pub mod identity;
pub mod encrypt;

pub use identity::KeyPair;
